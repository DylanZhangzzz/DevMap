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

fn parent_of<'a>(report: &'a GitRelationshipReport, branch: &str) -> Option<&'a str> {
    report
        .integration_branches
        .iter()
        .find(|candidate| candidate.name == branch)
        .and_then(|candidate| candidate.parent.as_deref())
}

fn target_of<'a>(
    report: &'a GitRelationshipReport,
    worktrees: &[WorktreeDescriptor],
    branch: &str,
) -> Option<&'a str> {
    row_for_branch(report, worktrees, branch)
        .merge_target
        .as_deref()
}

#[test]
fn hierarchy_routes_features_to_dev_and_dev_to_main() {
    let repo = support::committed_repo();
    let dev = support::linked_worktree(repo.path(), "dev");
    std::fs::write(dev.path().join("dev.txt"), "development\n").unwrap();
    support::git(dev.path(), ["add", "dev.txt"]);
    support::git(dev.path(), ["commit", "-m", "development base"]);
    let dylan = support::linked_worktree_from(repo.path(), "dylan_test", "dev");
    let joe = support::linked_worktree_from(repo.path(), "Joe_dev", "dev");

    let (report, worktrees) = report(repo.path());

    assert_eq!(parent_of(&report, "dev"), Some("main"));
    assert_eq!(target_of(&report, &worktrees, "dylan_test"), Some("dev"));
    assert_eq!(target_of(&report, &worktrees, "Joe_dev"), Some("dev"));
    assert_eq!(target_of(&report, &worktrees, "main"), None);

    drop((dylan, joe, dev));
}

#[test]
fn fork_point_contains_exact_commit_metadata_and_tags() {
    let repo = support::committed_repo();
    let initial = support::git(repo.path(), ["rev-parse", "HEAD"]);
    support::git(repo.path(), ["tag", "unrelated", initial.as_str()]);
    let dev = support::linked_worktree(repo.path(), "dev");
    std::fs::write(dev.path().join("base.txt"), "shared base\n").unwrap();
    support::git(dev.path(), ["add", "base.txt"]);
    support::git(
        dev.path(),
        [
            "commit",
            "--date",
            "2026-09-03T10:00:00+00:00",
            "-m",
            "shared development base",
        ],
    );
    let shared_base = support::git(dev.path(), ["rev-parse", "HEAD"]);
    support::git(repo.path(), ["tag", "z-shared", shared_base.as_str()]);
    support::git(repo.path(), ["tag", "a-shared", shared_base.as_str()]);
    let dylan = support::linked_worktree_from(repo.path(), "dylan_test", "dev");
    let joe = support::linked_worktree_from(repo.path(), "Joe_dev", "dev");
    std::fs::write(dev.path().join("advanced.txt"), "advanced\n").unwrap();
    support::git(dev.path(), ["add", "advanced.txt"]);
    support::git(dev.path(), ["commit", "-m", "advance development"]);

    let (report, worktrees) = report(repo.path());
    let dylan_fork = row_for_branch(&report, &worktrees, "dylan_test")
        .fork_point
        .as_ref()
        .unwrap();
    let joe_fork = row_for_branch(&report, &worktrees, "Joe_dev")
        .fork_point
        .as_ref()
        .unwrap();

    assert_eq!(dylan_fork.commit, shared_base);
    assert_eq!(dylan_fork, joe_fork);
    assert_eq!(dylan_fork.target_branch, "dev");
    assert_eq!(dylan_fork.tags, ["a-shared", "z-shared"]);
    assert_eq!(
        dylan_fork.subject.as_deref(),
        Some("shared development base")
    );
    assert_eq!(
        dylan_fork.authored_at.as_deref(),
        Some("2026-09-03T10:00:00Z")
    );
    assert_eq!(dylan_fork.distance_to_target, Some(1));

    drop((dylan, joe, dev));
}

#[test]
fn exact_tags_do_not_leak_from_other_commits() {
    let repo = support::committed_repo();
    let initial = support::git(repo.path(), ["rev-parse", "HEAD"]);
    support::git(repo.path(), ["tag", "initial-only", initial.as_str()]);
    let dev = support::linked_worktree(repo.path(), "dev");
    std::fs::write(dev.path().join("dev.txt"), "development\n").unwrap();
    support::git(dev.path(), ["add", "dev.txt"]);
    support::git(dev.path(), ["commit", "-m", "development base"]);
    let feature = support::linked_worktree_from(repo.path(), "feature", "dev");

    let (report, worktrees) = report(repo.path());
    let fork = row_for_branch(&report, &worktrees, "feature")
        .fork_point
        .as_ref()
        .unwrap();

    assert!(fork.tags.is_empty());

    drop((feature, dev));
}

#[test]
fn merge_base_failure_retains_unknown_workspace() {
    let repo = support::committed_repo();
    let feature = support::linked_worktree(repo.path(), "feature");
    std::fs::write(feature.path().join("dirty.txt"), "dirty\n").unwrap();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let mut worktrees = WorktreeScanner::scan(&workspace).unwrap();
    let feature_descriptor = worktrees
        .iter_mut()
        .find(|row| row.branch.as_deref() == Some("feature"))
        .unwrap();
    feature_descriptor.head = "0".repeat(40);
    let feature_id = feature_descriptor.worktree_id.clone();

    let report = GitRelationshipResolver::resolve(&workspace, &worktrees).unwrap();
    let relationship = report.by_worktree_id.get(&feature_id).unwrap();

    assert_eq!(relationship.fork_point, None);
    assert_eq!(relationship.merged, None);
    assert!(relationship.dirty);
    assert_eq!(relationship.changed_file_count, 1);
    assert!(report.warnings.iter().any(|warning| {
        warning.code == "git_merge_base_unavailable"
            && warning.worktree_id.as_deref() == Some(feature_id.as_str())
    }));

    drop(feature);
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
