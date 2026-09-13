use super::*;

// Frozen serial workspace assembly, including the original parsing fallback.
fn serial(inspector: &SourceGitInspector, head: String) -> Result<SourceWorkspace, DevMapError> {
    let paths = inspector.required_git([
        "rev-parse",
        "--show-toplevel",
        "--git-dir",
        "--git-common-dir",
    ])?;
    let [root, git_dir, git_common_dir] = match workspace_paths(&paths) {
        Some(paths) => paths.map(str::to_owned),
        None => [
            inspector.required_git(["rev-parse", "--show-toplevel"])?,
            inspector.required_git(["rev-parse", "--git-dir"])?,
            inspector.required_git(["rev-parse", "--git-common-dir"])?,
        ],
    };
    let branch = inspector.optional_git(["symbolic-ref", "--short", "-q", "HEAD"])?;
    let root = PathBuf::from(root);
    let resolve = |value: String| {
        let path = PathBuf::from(value);
        if path.is_absolute() {
            path
        } else {
            root.join(path)
        }
    };
    let git_dir = resolve(git_dir);
    let git_common_dir = crate::fs_security::checked_canonical_directory(&resolve(git_common_dir))?;
    Ok(SourceWorkspace {
        root,
        git_dir,
        git_common_dir,
        branch,
        head,
    })
}

fn run(root: &Path, args: &[&str]) {
    let output = git_at(root, args).unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn compare(root: &Path, unborn: bool) {
    let inspector = SourceGitInspector::open(root).unwrap();
    let head = if unborn {
        String::new()
    } else {
        inspector.required_git(["rev-parse", "HEAD"]).unwrap()
    };
    let expected = serial(&inspector, head.clone()).unwrap();
    assert_eq!(inspector.workspace_with_head(head).unwrap(), expected);
    assert_eq!(inspector.workspace_allow_unborn().unwrap(), expected);
    if !unborn {
        assert_eq!(inspector.workspace().unwrap(), expected);
    }
}

#[test]
fn workspace_overlap_matches_serial_main_linked_detached_and_unborn() {
    let temp = tempfile::tempdir().unwrap();
    let root = temp.path().join("main space é");
    fs::create_dir(&root).unwrap();
    run(&root, &["init", "--quiet", "-b", "main"]);
    compare(&root, true);
    run(
        &root,
        &[
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@example.invalid",
            "commit",
            "--allow-empty",
            "-qm",
            "initial",
        ],
    );
    compare(&root, false);
    let linked = temp.path().join("linked space é");
    run(
        &root,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "linked",
            linked.to_str().unwrap(),
        ],
    );
    compare(&linked, false);
    run(&linked, &["checkout", "--detach", "-q"]);
    compare(&linked, false);
    run(&root, &["checkout", "--orphan", "empty"]);
    compare(&root, true);
    // Both ordinary path failures retain the original error and command label.
    let inspector = SourceGitInspector::open(&linked).unwrap();
    fs::remove_file(linked.join(".git")).unwrap();
    assert_eq!(
        format!("{:?}", serial(&inspector, String::new()).unwrap_err()),
        format!(
            "{:?}",
            inspector.workspace_with_head(String::new()).unwrap_err()
        )
    );
}

use std::fs;
