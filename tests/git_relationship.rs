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

#[test]
fn target_error_precedes_unavailable_status_and_a_later_scan_remains_fresh() {
    let repo = support::committed_repo();
    let other = support::linked_worktree(repo.path(), "feature-unavailable");
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let worktrees = WorktreeScanner::scan(&workspace).unwrap();
    let unavailable = worktrees
        .iter()
        .find(|row| !row.is_current)
        .unwrap()
        .worktree_id
        .clone();
    support::git(
        repo.path(),
        ["worktree", "remove", other.path().to_str().unwrap()],
    );
    let blob = support::git(repo.path(), ["hash-object", "-w", "README.md"]);
    support::git(repo.path(), ["update-ref", "refs/tags/not-a-commit", &blob]);
    support::git(
        repo.path(),
        [
            "config",
            "devmap.developmentTarget",
            "refs/tags/not-a-commit",
        ],
    );
    let error = GitRelationshipResolver::resolve(&workspace, &worktrees).unwrap_err();
    match error {
        devmap::error::DevMapError::GitCommand { command, .. } => {
            assert_eq!(
                command,
                "git rev-parse --verify refs/tags/not-a-commit^{commit}"
            );
        }
        other => panic!("target failure must retain precedence over unavailable status: {other:?}"),
    }

    // The same observed inventory can contain a disappearing root. Its status
    // error stays local, while the next operation must reread target and dirt.
    support::git(
        repo.path(),
        ["config", "--unset", "devmap.developmentTarget"],
    );
    std::fs::write(
        repo.path().join("new-dirty.txt"),
        "changed after failed scan\n",
    )
    .unwrap();
    let report = GitRelationshipResolver::resolve(&workspace, &worktrees).unwrap();
    assert_eq!(
        report.target.as_ref().unwrap().source,
        TargetSource::LocalMain
    );
    let absent = &report.by_worktree_id[&unavailable];
    assert!(!absent.status_observed);
    assert_eq!(absent.merged, None);
    assert!(
        report
            .warnings
            .iter()
            .any(|warning| warning.code == "git_relationship_unavailable"
                && warning.worktree_id.as_ref() == Some(&unavailable))
    );
    let current = worktrees.iter().find(|row| row.is_current).unwrap();
    assert!(report.by_worktree_id[&current.worktree_id].status_observed);
    assert!(report.by_worktree_id[&current.worktree_id].dirty);
    assert_eq!(
        report.by_worktree_id[&current.worktree_id].changed_file_count,
        1
    );
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
fn unused_broken_develop_probe_does_not_override_selected_dev() {
    let repo = support::committed_repo();
    support::git(repo.path(), ["branch", "dev"]);
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let worktrees = WorktreeScanner::scan(&workspace).unwrap();
    let broken_path = workspace.git_common_dir.join("refs/heads/develop");
    let broken_bytes = format!("{}\n", "a".repeat(40)).into_bytes();
    std::fs::write(&broken_path, &broken_bytes).unwrap();
    // Establish that this is a real ordinary probe error, not an absent ref.
    let probe = std::process::Command::new("git")
        .arg("-C")
        .arg(repo.path())
        .args(["show-ref", "--verify", "--quiet", "refs/heads/develop"])
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_NO_LAZY_FETCH", "1")
        .env("GIT_NO_REPLACE_OBJECTS", "1")
        .output()
        .unwrap();
    assert_eq!(
        probe.status.code(),
        Some(128),
        "fixture must exercise GitCommand error: {probe:?}"
    );
    let report = GitRelationshipResolver::resolve(&workspace, &worktrees).unwrap();
    assert_eq!(report.target.as_ref().unwrap().name, "dev");
    assert_eq!(
        report.target.as_ref().unwrap().source,
        TargetSource::LocalDev
    );
    assert!(report.warnings.is_empty());
    assert_eq!(std::fs::read(broken_path).unwrap(), broken_bytes);
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

#[test]
fn failed_status_is_retained_as_unknown_instead_of_clean() {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let mut worktrees = WorktreeScanner::scan(&workspace).unwrap();
    worktrees[0].root = repo.path().join("missing-worktree");
    let worktree_id = worktrees[0].worktree_id.clone();

    let report = GitRelationshipResolver::resolve(&workspace, &worktrees).unwrap();
    let relationship = &report.by_worktree_id[&worktree_id];

    assert!(!relationship.status_observed);
    assert!(!relationship.dirty);
    assert_eq!(relationship.changed_file_count, 0);
    assert!(report.warnings.iter().any(|warning| {
        warning.code == "git_relationship_unavailable"
            && warning.worktree_id.as_deref() == Some(worktree_id.as_str())
    }));
}

fn commit_node(repo: &std::path::Path, parents: &[&str], message: &str) -> String {
    let tree = support::git(repo, ["rev-parse", "HEAD^{tree}"]);
    let mut args = vec!["commit-tree", tree.as_str(), "-m", message];
    for parent in parents {
        args.extend(["-p", *parent]);
    }
    support::git(repo, args)
}

#[test]
fn ancestor_distance_matches_git_when_target_is_a_later_commit() {
    let repo = support::committed_repo();
    let head = support::git(repo.path(), ["rev-parse", "HEAD"]);
    let feature = support::linked_worktree(repo.path(), "feature");
    let target = commit_node(repo.path(), &[&head], "later target");
    support::git(repo.path(), ["update-ref", "refs/heads/main", &target]);
    let expected = support::git(
        repo.path(),
        ["rev-list", "--count", &format!("{head}..{target}")],
    );
    let (report, worktrees) = report(repo.path());
    let row = row_for_branch(&report, &worktrees, "feature");
    assert_eq!((row.ahead, row.behind), (Some(0), Some(1)));
    let fork = row.fork_point.as_ref().unwrap();
    assert_eq!(fork.commit, head);
    assert_eq!(fork.distance_to_target, Some(expected.parse().unwrap()));
    drop(feature);
}

#[test]
fn divergent_and_criss_cross_distance_preserve_selected_merge_base_oracle() {
    for criss_cross in [false, true] {
        let repo = support::committed_repo();
        let initial = support::git(repo.path(), ["rev-parse", "HEAD"]);
        let left = commit_node(repo.path(), &[&initial], "left");
        let right = commit_node(repo.path(), &[&initial], "right");
        let (target, head) = if criss_cross {
            (
                commit_node(repo.path(), &[&left, &right], "left merge"),
                commit_node(repo.path(), &[&right, &left], "right merge"),
            )
        } else {
            (left, right)
        };
        support::git(repo.path(), ["update-ref", "refs/heads/main", &target]);
        let feature = support::linked_worktree_from(repo.path(), "feature", &head);
        let bases = support::git(feature.path(), ["merge-base", "--all", &target, &head]);
        assert_eq!(bases.lines().count(), if criss_cross { 2 } else { 1 });
        let (report, worktrees) = report(repo.path());
        let row = row_for_branch(&report, &worktrees, "feature");
        let fork = row.fork_point.as_ref().unwrap();
        assert_ne!(fork.commit, head);
        assert!(bases.lines().any(|base| base == fork.commit));
        let expected = support::git(
            feature.path(),
            ["rev-list", "--count", &format!("{}..{target}", fork.commit)],
        );
        assert_eq!(fork.distance_to_target, Some(expected.parse().unwrap()));
        assert_eq!(row.ahead, Some(1));
        assert_eq!(row.behind, Some(1));
        if criss_cross {
            assert_eq!(fork.distance_to_target, Some(2));
            assert_ne!(fork.distance_to_target, row.behind);
        }
    }
}
