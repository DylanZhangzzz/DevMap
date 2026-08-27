use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use tempfile::TempDir;

pub fn git<I, S>(root: &Path, args: I) -> String
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let output = git_output(root, args);
    assert!(
        output.status.success(),
        "git failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("git output is UTF-8")
        .trim()
        .to_owned()
}

pub fn git_output<I, S>(root: &Path, args: I) -> Output
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    Command::new("git")
        .args(args)
        .current_dir(root)
        .output()
        .expect("run git")
}

#[allow(dead_code)]
pub fn committed_repo() -> TempDir {
    let directory = tempfile::tempdir().expect("create temporary repository");
    git(directory.path(), ["init", "-b", "main"]);
    git(directory.path(), ["config", "user.name", "DevMap Test"]);
    git(
        directory.path(),
        ["config", "user.email", "devmap-test@example.test"],
    );
    fs::write(directory.path().join("README.md"), "# Fixture\n").expect("write fixture");
    git(directory.path(), ["add", "--", "README.md"]);
    git(directory.path(), ["commit", "-m", "initial fixture"]);
    directory
}

#[allow(dead_code)]
pub fn linked_worktree(repo: &Path, branch: &str) -> TempDir {
    let directory = tempfile::tempdir().expect("create linked worktree directory");
    git(
        repo,
        [
            "worktree",
            "add",
            "-b",
            branch,
            directory.path().to_str().expect("worktree path is UTF-8"),
        ],
    );
    directory
}
