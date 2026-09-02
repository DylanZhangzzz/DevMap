mod support;

use std::fs;

use devmap::git::SourceGitInspector;
use devmap::worktrees::WorktreeScanner;

#[test]
fn linked_worktrees_share_common_dir_but_have_distinct_ids() {
    let repo = support::committed_repo();
    let linked = support::linked_worktree(repo.path(), "codex/dock-agent");

    let main = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let other = SourceGitInspector::open(linked.path())
        .unwrap()
        .workspace()
        .unwrap();

    assert_eq!(main.git_common_dir, other.git_common_dir);
    let rows = WorktreeScanner::scan(&main).unwrap();
    assert_eq!(rows.len(), 2);
    assert_eq!(rows.iter().filter(|row| row.is_current).count(), 1);
    assert_ne!(rows[0].worktree_id, rows[1].worktree_id);
}

#[test]
fn scanner_preserves_detached_locked_and_space_path_state() {
    let repo = support::committed_repo();
    let parent = tempfile::tempdir().unwrap();
    let linked_path = parent.path().join("agent worktree with spaces");
    support::git(
        repo.path(),
        ["worktree", "add", "--detach", linked_path.to_str().unwrap()],
    );
    support::git(
        repo.path(),
        [
            "worktree",
            "lock",
            "--reason",
            "agent is active",
            linked_path.to_str().unwrap(),
        ],
    );

    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let rows = WorktreeScanner::scan(&workspace).unwrap();
    let linked = rows.iter().find(|row| row.root == linked_path).unwrap();

    assert_eq!(linked.branch, None);
    assert!(linked.is_locked);
    assert!(!linked.is_prunable);
}

#[test]
fn scanner_keeps_a_missing_registered_worktree_as_prunable() {
    let repo = support::committed_repo();
    let linked = support::linked_worktree(repo.path(), "codex/prunable-agent");
    let missing_root = linked.path().to_path_buf();
    drop(linked);

    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let rows = WorktreeScanner::scan(&workspace).unwrap();
    let missing = rows.iter().find(|row| row.root == missing_root).unwrap();

    assert!(missing.is_prunable);
    assert!(!missing.is_current);
}

#[test]
fn scanner_rejects_a_linked_worktree_dot_git_symlink_when_supported() {
    let repo = support::committed_repo();
    let linked = support::linked_worktree(repo.path(), "codex/symlink-agent");
    let dot_git = linked.path().join(".git");
    let real_dot_git = linked.path().join("real-dot-git");
    fs::rename(&dot_git, &real_dot_git).unwrap();

    #[cfg(unix)]
    let link_result = std::os::unix::fs::symlink(&real_dot_git, &dot_git);
    #[cfg(windows)]
    let link_result = std::os::windows::fs::symlink_file(&real_dot_git, &dot_git);
    if let Err(error) = link_result {
        if error.kind() == std::io::ErrorKind::PermissionDenied
            || error.raw_os_error() == Some(1314)
        {
            return;
        }
        panic!("create .git symlink: {error}");
    }

    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let error = WorktreeScanner::scan(&workspace).unwrap_err();
    assert!(matches!(
        error,
        devmap::error::DevMapError::UnsafeInstallerOverwrite(path) if path == dot_git
    ));
}
