#![allow(dead_code)]

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use tempfile::TempDir;

pub const SCENARIO_SESSION: &str = "phase-1b-session";
pub const SCENARIO_ROUTE: &str = "route-native-capture";
pub const SCENARIO_MAIN_AGENT: &str = "agent-main";
pub const SCENARIO_CHILD_AGENT: &str = "agent-child";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitMetadataSnapshot {
    pub head: String,
    pub branch: String,
    pub index: String,
    pub refs: String,
    pub config: String,
    pub stash: String,
    pub remotes: String,
    pub worktrees: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceSnapshot {
    pub files: BTreeMap<PathBuf, Vec<u8>>,
    pub git: GitMetadataSnapshot,
}

pub fn source_snapshot(root: &Path) -> SourceSnapshot {
    let mut files = BTreeMap::new();
    collect_source_files(root, root, &mut files);
    SourceSnapshot {
        files,
        git: GitMetadataSnapshot {
            head: git(root, ["rev-parse", "HEAD"]),
            branch: git(root, ["branch", "--show-current"]),
            index: git(root, ["ls-files", "--stage"]),
            refs: git(root, ["for-each-ref", "--format=%(refname):%(objectname)"]),
            config: git(root, ["config", "--local", "--list", "--show-origin"]),
            stash: git(root, ["stash", "list", "--format=%gd:%H"]),
            remotes: git(root, ["remote", "-v"]),
            worktrees: git(root, ["worktree", "list", "--porcelain"]),
        },
    }
}

pub fn assert_only_source_paths_changed(
    before: &SourceSnapshot,
    after: &SourceSnapshot,
    expected_paths: &[&Path],
) {
    assert_eq!(after.git, before.git, "source Git metadata changed");
    let changed = before
        .files
        .keys()
        .chain(after.files.keys())
        .filter(|path| before.files.get(*path) != after.files.get(*path))
        .cloned()
        .collect::<std::collections::BTreeSet<_>>();
    let expected = expected_paths
        .iter()
        .map(|path| path.to_path_buf())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(changed, expected, "unexpected source-root file changes");
}

fn collect_source_files(root: &Path, directory: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
    for entry in fs::read_dir(directory).expect("read source directory") {
        let entry = entry.expect("read source entry");
        let path = entry.path();
        let relative = path.strip_prefix(root).expect("source-relative path");
        if relative.starts_with(".git") {
            continue;
        }
        if path.is_dir() {
            collect_source_files(root, &path, files);
        } else {
            files.insert(
                relative.to_path_buf(),
                fs::read(path).expect("read source file"),
            );
        }
    }
}

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
