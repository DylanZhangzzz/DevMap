mod support;

use std::fs;

use devmap::context::ContextRepo;
use serde_json::json;
use support::git;

#[test]
fn creation_uses_ordinary_main_and_repository_local_bot_identity() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("project-context");

    let context = ContextRepo::create(&root).unwrap();

    assert_eq!(context.root(), root.as_path());
    assert_eq!(git(&root, ["branch", "--show-current"]), "main");
    assert_eq!(git(&root, ["config", "--local", "user.name"]), "DevMap Bot");
    assert_eq!(
        git(&root, ["config", "--local", "user.email"]),
        "devmap-bot@localhost"
    );
    assert!(root.join(".devmap-context.json").is_file());

    let refs = git(&root, ["for-each-ref", "--format=%(refname)"]);
    assert!(!refs.contains("refs/devmap"));
    assert!(!refs.contains("refs/notes"));
}

#[test]
fn canonical_objects_are_content_addressed_and_committed_explicitly() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("project-context");
    let context = ContextRepo::create(&root).unwrap();

    let stored = context
        .write_canonical("common-ground", &json!({"z": 2, "a": 1}))
        .unwrap();

    assert!(stored.id.starts_with("common-ground:sha256-"));
    assert!(stored.relative_path.starts_with("objects/common-ground/"));
    assert_eq!(
        fs::read(context.root().join(&stored.relative_path)).unwrap(),
        br#"{"a":1,"z":2}"#
    );

    let commit = context.commit_all("store fixture object").unwrap();
    assert_eq!(commit, git(&root, ["rev-parse", "HEAD"]));
    assert!(git(&root, ["status", "--porcelain=v1"]).is_empty());
}

#[test]
fn commit_refuses_unexpected_context_files() {
    let parent = tempfile::tempdir().unwrap();
    let root = parent.path().join("project-context");
    let context = ContextRepo::create(&root).unwrap();
    context
        .write_canonical("approval", &json!({"actor": "Dylan"}))
        .unwrap();
    fs::write(root.join("unexpected.txt"), "not owned by DevMap\n").unwrap();

    let error = context
        .commit_all("must not commit everything")
        .unwrap_err();

    assert!(error.to_string().contains("unexpected.txt"));
    assert!(
        git(&root, ["status", "--porcelain=v1"])
            .lines()
            .any(|line| line.ends_with("unexpected.txt"))
    );
}
