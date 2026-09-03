mod support;

use devmap::git::SourceGitInspector;
use devmap::git_relationship::{GitRelationshipReport, GitRelationshipResolver, TargetSource};
use devmap::worktrees::{WorktreeDescriptor, WorktreeScanner};

fn report(repo: &std::path::Path) -> (GitRelationshipReport, Vec<WorktreeDescriptor>) {
    let workspace = SourceGitInspector::open(repo).unwrap().workspace().unwrap();
    let worktrees = WorktreeScanner::scan(&workspace).unwrap();
    let report = GitRelationshipResolver::resolve(&workspace, &worktrees).unwrap();
    (report, worktrees)
}

fn row_for_branch<'a>(
    report: &'a GitRelationshipReport,
    worktrees: &[WorktreeDescriptor],
    branch: &str,
) -> &'a devmap::git_relationship::GitRelationship {
    let worktree = worktrees
        .iter()
        .find(|row| row.branch.as_deref() == Some(branch))
        .unwrap();
    report.by_worktree_id.get(&worktree.worktree_id).unwrap()
}

#[test]
fn configured_target_wins_over_dev_and_remote_default() {
    let repo = support::committed_repo();
    support::git(repo.path(), ["branch", "dev"]);
    support::git(repo.path(), ["branch", "release"]);
    support::git(
        repo.path(),
        ["config", "devmap.developmentTarget", "release"],
    );

    let (report, _) = report(repo.path());

    let target = report.target.unwrap();
    assert_eq!(target.name, "release");
    assert_eq!(target.ref_name, "refs/heads/release");
    assert_eq!(target.source, TargetSource::Config);
}

#[test]
fn local_dev_wins_over_develop_and_main() {
    let repo = support::committed_repo();
    support::git(repo.path(), ["branch", "develop"]);
    support::git(repo.path(), ["branch", "dev"]);

    let (report, _) = report(repo.path());

    let target = report.target.unwrap();
    assert_eq!(target.name, "dev");
    assert_eq!(target.ref_name, "refs/heads/dev");
    assert_eq!(target.source, TargetSource::LocalDev);
}

#[test]
fn invalid_configured_target_warns_and_falls_back_to_main() {
    let repo = support::committed_repo();
    support::git(
        repo.path(),
        ["config", "devmap.developmentTarget", "bad..target"],
    );

    let (report, _) = report(repo.path());

    let target = report.target.unwrap();
    assert_eq!(target.name, "main");
    assert_eq!(target.source, TargetSource::LocalMain);
    assert_eq!(
        report.warnings[0].code,
        "configured_development_target_unavailable"
    );
}

#[test]
fn relationship_distinguishes_merged_unmerged_and_dirty_worktrees() {
    let repo = support::committed_repo();
    let merged = support::linked_worktree(repo.path(), "codex/merged");
    std::fs::write(merged.path().join("merged.txt"), "merged\n").unwrap();
    support::git(merged.path(), ["add", "merged.txt"]);
    support::git(merged.path(), ["commit", "-m", "merged work"]);
    support::git(repo.path(), ["merge", "--ff-only", "codex/merged"]);

    let open = support::linked_worktree(repo.path(), "codex/open");
    std::fs::write(open.path().join("open.txt"), "open\n").unwrap();
    support::git(open.path(), ["add", "open.txt"]);
    support::git(open.path(), ["commit", "-m", "open work"]);
    std::fs::write(open.path().join("dirty.txt"), "dirty\n").unwrap();

    let (report, worktrees) = report(repo.path());
    let merged_row = row_for_branch(&report, &worktrees, "codex/merged");
    let open_row = row_for_branch(&report, &worktrees, "codex/open");

    assert_eq!(
        (merged_row.merged, merged_row.ahead, merged_row.behind),
        (Some(true), Some(0), Some(0))
    );
    assert_eq!(
        (open_row.merged, open_row.ahead, open_row.behind),
        (Some(false), Some(1), Some(0))
    );
    assert_eq!((open_row.dirty, open_row.changed_file_count), (true, 1));
}

#[test]
fn dirty_count_treats_a_rename_as_one_changed_file() {
    let repo = support::committed_repo();
    let worktree = support::linked_worktree(repo.path(), "codex/rename");
    support::git(worktree.path(), ["mv", "README.md", "RENAMED.md"]);

    let (report, worktrees) = report(repo.path());
    let row = row_for_branch(&report, &worktrees, "codex/rename");

    assert!(row.dirty);
    assert_eq!(row.changed_file_count, 1);
}
