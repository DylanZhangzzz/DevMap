mod support;

use std::fs;

use devmap::git::SourceGitInspector;
use support::{committed_repo, git};

fn repository_snapshot(root: &std::path::Path) -> Vec<String> {
    vec![
        git(root, ["rev-parse", "HEAD"]),
        git(root, ["status", "--porcelain=v1"]),
        git(root, ["for-each-ref", "--format=%(refname):%(objectname)"]),
        git(root, ["config", "--local", "--list"]),
    ]
}

#[test]
fn inspection_is_read_only_and_records_the_adoption_anchor() {
    let repository = committed_repo();
    git(
        repository.path(),
        [
            "remote",
            "add",
            "origin",
            "https://example.test/acme/payments.git",
        ],
    );
    fs::write(repository.path().join("uncommitted.txt"), "local work\n").unwrap();
    fs::create_dir(repository.path().join("nested")).unwrap();

    let before = repository_snapshot(repository.path());
    let anchor = SourceGitInspector::open(repository.path().join("nested"))
        .unwrap()
        .inspect()
        .unwrap();
    let after = repository_snapshot(repository.path());

    assert_eq!(before, after, "inspection changed the source repository");
    assert_eq!(
        anchor.head_commit,
        git(repository.path(), ["rev-parse", "HEAD"])
    );
    assert_eq!(anchor.default_branch.as_deref(), Some("main"));
    assert_eq!(
        anchor.remote_url.as_deref(),
        Some("https://example.test/acme/payments.git")
    );
    assert!(anchor.dirty_at_adoption);
    assert!(anchor.repository_fingerprint.starts_with("sha256-"));
}

#[test]
fn inspection_rejects_non_repository_and_unborn_repository() {
    let non_repository = tempfile::tempdir().unwrap();
    let error = SourceGitInspector::open(non_repository.path()).unwrap_err();
    assert!(error.to_string().contains("Git repository"));

    let unborn = tempfile::tempdir().unwrap();
    git(unborn.path(), ["init", "-b", "main"]);
    let error = SourceGitInspector::open(unborn.path())
        .unwrap()
        .inspect()
        .unwrap_err();
    assert!(error.to_string().contains("HEAD"));
}
