use super::super::{
    GitRelationshipReport, SharedFactsTestMode, SharedFactsTestStats,
    resolve_with_shared_facts_test,
};

mod fault_tests;
use crate::{git::SourceGitInspector, worktrees::WorktreeScanner};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const CASE: &str = "DEVMAP_SHARED_FACTS_INTEGRATION_CHILD";
const ROOT: &str = "DEVMAP_SHARED_FACTS_INTEGRATION_ROOT";

struct OwnedChild(Child);
impl Drop for OwnedChild {
    fn drop(&mut self) {
        if !matches!(self.0.try_wait(), Ok(Some(_))) {
            let _ = self.0.kill();
        }
        let _ = self.0.wait();
    }
}

fn git(root: &Path, args: &[&str]) -> String {
    let output = super::super::git_output(root, args).unwrap();
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .trim_end()
        .to_owned()
}

fn encoded(report: &GitRelationshipReport) -> Vec<u8> {
    serde_json::to_vec(&serde_json::json!({
        "target": report.target,
        "integration_branches": report.integration_branches,
        "by_worktree_id": report.by_worktree_id,
        "warnings": report.warnings.iter().map(|warning| serde_json::json!({
            "code": warning.code, "worktree_id": warning.worktree_id
        })).collect::<Vec<_>>()
    }))
    .unwrap()
}

fn assert_shared(stats: &SharedFactsTestStats) {
    assert_eq!(
        stats.shared_groups, 1,
        "same immutable fork pair must execute one shared group"
    );
    assert!(
        stats.shared_rows >= 2,
        "real shared branch must supply both linked roots"
    );
    assert!(
        stats.original_rows <= 1,
        "linked roots must not silently use original computation"
    );
}

#[test]
fn shared_reports_match_original_with_distinct_status_and_refresh_freshness() {
    if std::env::var_os(CASE).is_none() {
        let owned = tempfile::tempdir().unwrap();
        let home = owned.path().join("home");
        let xdg = owned.path().join("xdg");
        fs::create_dir(&home).unwrap();
        fs::create_dir(&xdg).unwrap();
        let stdout = owned.path().join("stdout");
        let stderr = owned.path().join("stderr");
        let mut command = Command::new(std::env::current_exe().unwrap());
        command.args(["--exact", "git_relationship::shared_facts::integration_tests::shared_reports_match_original_with_distinct_status_and_refresh_freshness", "--nocapture", "--test-threads=1"]);
        for (key, _) in std::env::vars_os() {
            if key
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(key);
            }
        }
        command
            .env(CASE, "1")
            .env(ROOT, owned.path())
            .env("HOME", &home)
            .env("USERPROFILE", &home)
            .env("XDG_CONFIG_HOME", &xdg)
            .stdin(Stdio::null())
            .stdout(fs::File::create(&stdout).unwrap())
            .stderr(fs::File::create(&stderr).unwrap());
        let mut child = OwnedChild(command.spawn().unwrap());
        let deadline = Instant::now() + Duration::from_secs(90);
        let status = loop {
            if let Some(status) = child.0.try_wait().unwrap() {
                break status;
            }
            assert!(
                Instant::now() < deadline,
                "owned sharing test child timed out"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(
            status.success(),
            "{status}\n{}\n{}",
            fs::read_to_string(stdout).unwrap(),
            fs::read_to_string(stderr).unwrap()
        );
        return;
    }
    let owned = std::path::PathBuf::from(std::env::var_os(ROOT).unwrap());
    let main = owned.join("main");
    let a = owned.join("a");
    let b = owned.join("b");
    fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    fs::write(main.join("tracked.txt"), "base\n").unwrap();
    git(&main, &["add", "tracked.txt"]);
    git(
        &main,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "-qm",
            "shared base",
        ],
    );
    git(&main, &["branch", "dev"]);
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "topic-a",
            a.to_str().unwrap(),
        ],
    );
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "topic-b",
            b.to_str().unwrap(),
        ],
    );
    fs::write(a.join("dirty.txt"), "only a\n").unwrap();
    git(&b, &["mv", "tracked.txt", "renamed.txt"]);
    let workspace = SourceGitInspector::open(&main)
        .unwrap()
        .workspace()
        .unwrap();
    let worktrees = WorktreeScanner::scan(&workspace).unwrap();
    assert_eq!(worktrees.len(), 3);
    assert!(worktrees.iter().all(|row| row.head == workspace.head));
    let (original, _) =
        resolve_with_shared_facts_test(&workspace, &worktrees, SharedFactsTestMode::Original)
            .unwrap();
    let (shared, stats) =
        resolve_with_shared_facts_test(&workspace, &worktrees, SharedFactsTestMode::Shared)
            .unwrap();
    assert_eq!(
        encoded(&shared),
        encoded(&original),
        "all relationship fields and warnings must match original"
    );
    for row in &worktrees {
        let relationship = &shared.by_worktree_id[&row.worktree_id];
        assert!(relationship.status_observed);
        assert_eq!(
            relationship.changed_file_count,
            if row.is_current { 0 } else { 1 }
        );
        assert_eq!(relationship.dirty, !row.is_current);
    }
    assert_shared(&stats);
    let previous = encoded(&shared);
    fs::write(a.join("another-dirty.txt"), "later a\n").unwrap();
    git(&main, &["tag", "created-between-operations"]);
    let (fresh_original, _) =
        resolve_with_shared_facts_test(&workspace, &worktrees, SharedFactsTestMode::Original)
            .unwrap();
    let (fresh_shared, fresh_stats) =
        resolve_with_shared_facts_test(&workspace, &worktrees, SharedFactsTestMode::Shared)
            .unwrap();
    assert_eq!(encoded(&fresh_shared), encoded(&fresh_original));
    assert_ne!(
        encoded(&fresh_shared),
        previous,
        "next operation must reread status and tag refs"
    );
    assert_shared(&fresh_stats);
    let a_row = worktrees
        .iter()
        .find(|row| row.branch.as_deref() == Some("topic-a"))
        .unwrap();
    assert_eq!(
        fresh_shared.by_worktree_id[&a_row.worktree_id].changed_file_count,
        2
    );
    assert!(
        fresh_shared.by_worktree_id[&a_row.worktree_id]
            .fork_point
            .as_ref()
            .unwrap()
            .tags
            .iter()
            .any(|tag| tag == "created-between-operations")
    );
}

fn isolated_case(name: &str) -> Option<std::path::PathBuf> {
    if std::env::var(CASE).as_deref() == Ok(name) {
        return Some(std::env::var_os(ROOT).unwrap().into());
    }
    let owned = tempfile::tempdir().unwrap();
    let home = owned.path().join("home");
    let xdg = owned.path().join("xdg");
    fs::create_dir(&home).unwrap();
    fs::create_dir(&xdg).unwrap();
    let stdout = owned.path().join("stdout");
    let stderr = owned.path().join("stderr");
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args([
        "--exact",
        &format!("git_relationship::shared_facts::integration_tests::{name}"),
        "--nocapture",
        "--test-threads=1",
    ]);
    for (key, _) in std::env::vars_os() {
        if key
            .to_string_lossy()
            .to_ascii_uppercase()
            .starts_with("GIT_")
        {
            command.env_remove(key);
        }
    }
    command
        .env(CASE, name)
        .env(ROOT, owned.path())
        .env("HOME", &home)
        .env("USERPROFILE", &home)
        .env("XDG_CONFIG_HOME", &xdg)
        .stdin(Stdio::null())
        .stdout(fs::File::create(&stdout).unwrap())
        .stderr(fs::File::create(&stderr).unwrap());
    let mut child = OwnedChild(command.spawn().unwrap());
    let deadline = Instant::now() + Duration::from_secs(180);
    let status = loop {
        if let Some(status) = child.0.try_wait().unwrap() {
            break status;
        }
        assert!(
            Instant::now() < deadline,
            "owned differential child timed out"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    assert!(
        status.success(),
        "{status}\n{}\n{}",
        fs::read_to_string(stdout).unwrap(),
        fs::read_to_string(stderr).unwrap()
    );
    None
}
fn commit(root: &Path, subject: &str) {
    git(
        root,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            subject,
        ],
    );
}
fn compare(
    root: &Path,
) -> (
    GitRelationshipReport,
    SharedFactsTestStats,
    Vec<crate::worktrees::WorktreeDescriptor>,
) {
    let workspace = SourceGitInspector::open(root).unwrap().workspace().unwrap();
    let rows = WorktreeScanner::scan(&workspace).unwrap();
    let (original, original_stats) =
        resolve_with_shared_facts_test(&workspace, &rows, SharedFactsTestMode::Original).unwrap();
    let (shared, stats) =
        resolve_with_shared_facts_test(&workspace, &rows, SharedFactsTestMode::Shared).unwrap();
    assert_eq!(
        encoded(&shared),
        encoded(&original),
        "full graph, metadata, labels and warnings differential"
    );
    assert_eq!(original_stats.shared_groups, 0);
    assert_eq!(original_stats.shared_rows, 0);
    assert_eq!(original_stats.original_rows, rows.len());
    (shared, stats, rows)
}

#[test]
fn shared_alias_targets_preserve_labels_tag_limit_and_unicode_metadata() {
    let Some(owned) =
        isolated_case("shared_alias_targets_preserve_labels_tag_limit_and_unicode_metadata")
    else {
        return;
    };
    let main = owned.join("main");
    let dev = owned.join("dev");
    let detached = owned.join("detached");
    fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    let subject = "共享分叉 — café 🚀";
    commit(&main, subject);
    git(&main, &["branch", "dev"]);
    for n in 0..40 {
        git(&main, &["tag", &format!("tag-{n:02}")]);
    }
    git(
        &main,
        &[
            "-c",
            "user.name=fixture",
            "-c",
            "user.email=fixture@example.invalid",
            "tag",
            "-a",
            "zz-annotated",
            "-m",
            "annotated",
        ],
    );
    git(
        &main,
        &["worktree", "add", "-q", dev.to_str().unwrap(), "dev"],
    );
    git(
        &main,
        &[
            "worktree",
            "add",
            "-q",
            "--detach",
            detached.to_str().unwrap(),
            "HEAD",
        ],
    );
    let (report, stats, rows) = compare(&main);
    assert_shared(&stats);
    let dev_row = rows
        .iter()
        .find(|r| r.branch.as_deref() == Some("dev"))
        .unwrap();
    let detached_row = rows.iter().find(|r| r.branch.is_none()).unwrap();
    assert_eq!(dev_row.head, detached_row.head);
    for (row, label) in [(dev_row, "main"), (detached_row, "dev")] {
        let facts = &report.by_worktree_id[&row.worktree_id];
        assert_eq!(facts.base_target.as_deref(), Some(label));
        assert_eq!(facts.merge_target.as_deref(), Some(label));
        let fork = facts.fork_point.as_ref().unwrap();
        assert_eq!(fork.target_branch, label);
        assert_eq!(fork.subject.as_deref(), Some(subject));
        assert_eq!(
            fork.tags,
            (0..32).map(|n| format!("tag-{n:02}")).collect::<Vec<_>>()
        );
        assert_eq!(facts.ahead, Some(0));
        assert_eq!(facts.behind, Some(0));
    }
}

#[test]
fn shared_graph_shapes_and_unsupported_config_use_exact_original_fallback() {
    let Some(owned) =
        isolated_case("shared_graph_shapes_and_unsupported_config_use_exact_original_fallback")
    else {
        return;
    };
    let main = owned.join("main");
    let a = owned.join("a");
    let b = owned.join("b");
    fs::create_dir(&main).unwrap();
    git(&main, &["init", "-q", "-b", "main"]);
    commit(&main, "base");
    let base = git(&main, &["rev-parse", "HEAD"]);
    git(&main, &["checkout", "-qb", "dev"]);
    commit(&main, "development child");
    git(&main, &["checkout", "-q", "main"]);
    for path in [&a, &b] {
        git(
            &main,
            &[
                "worktree",
                "add",
                "-q",
                "--detach",
                path.to_str().unwrap(),
                &base,
            ],
        );
    }
    let (ancestor, stats, rows) = compare(&main);
    assert_shared(&stats);
    for row in rows.iter().filter(|r| r.branch.is_none()) {
        let facts = &ancestor.by_worktree_id[&row.worktree_id];
        assert_eq!(facts.merged, Some(true));
        assert_eq!(facts.ahead, Some(0));
        assert_eq!(facts.behind, Some(1));
        assert_eq!(
            facts.fork_point.as_ref().unwrap().distance_to_target,
            Some(1)
        );
    }
    commit(&a, "independent topic child");
    let topic = git(&a, &["rev-parse", "HEAD"]);
    git(&b, &["reset", "--hard", &topic]);
    let (divergent, stats, rows) = compare(&main);
    assert_shared(&stats);
    for row in rows.iter().filter(|r| r.branch.is_none()) {
        let facts = &divergent.by_worktree_id[&row.worktree_id];
        assert_eq!(facts.merged, Some(false));
        assert_eq!(facts.ahead, Some(1));
        assert_eq!(facts.behind, Some(1));
        assert_eq!(facts.fork_point.as_ref().unwrap().commit, base);
    }
    // Unsupported but valid config must execute original rows, not just happen
    // to generate identical reports from a still-shared implementation.
    git(&main, &["config", "tag.sort", "-refname"]);
    let (_, fallback, rows) = compare(&main);
    assert_eq!(fallback.shared_groups, 0);
    assert_eq!(fallback.shared_rows, 0);
    assert_eq!(fallback.original_rows, rows.len());
    git(&main, &["config", "--unset", "tag.sort"]);
    git(&a, &["checkout", "--orphan", "unrelated"]);
    commit(&a, "unrelated root");
    let unrelated = git(&a, &["rev-parse", "HEAD"]);
    git(&a, &["checkout", "--detach", &unrelated]);
    git(&b, &["reset", "--hard", &unrelated]);
    let (report, fallback, rows) = compare(&main);
    assert_eq!(fallback.shared_groups, 0);
    assert_eq!(fallback.shared_rows, 0);
    assert_eq!(fallback.original_rows, rows.len());
    for row in rows.iter().filter(|r| r.branch.is_none()) {
        let facts = &report.by_worktree_id[&row.worktree_id];
        assert!(facts.fork_point.is_none());
        assert_eq!(facts.merged, None);
        assert!(report.warnings.iter().any(|w| w.worktree_id.as_deref()
            == Some(row.worktree_id.as_str())
            && w.code == "git_merge_base_unavailable"));
    }
}
