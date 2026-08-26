mod support;

use std::ffi::OsString;
use std::fs;

use devmap::run;
use support::{committed_repo, git, git_output};

#[test]
fn init_creates_a_reviewable_draft_without_mutating_source() {
    let source = committed_repo();
    fs::create_dir(source.path().join("docs")).unwrap();
    fs::write(
        source.path().join("docs/spec.md"),
        "# Product\nGeneral notes.\n\n## Payment Lock\nUse a PostgreSQL advisory lock.\n\n## Other\nDo something else.\n",
    )
    .unwrap();
    fs::write(
        source.path().join("docs/unrelated.md"),
        "SECRET TEXT THAT MUST NOT BE CAPTURED\n",
    )
    .unwrap();
    git(source.path(), ["add", "--", "docs"]);
    git(source.path(), ["commit", "-m", "add requirements"]);
    fs::write(source.path().join("local-work.txt"), "uncommitted\n").unwrap();

    let parent = tempfile::tempdir().unwrap();
    let context = parent.path().join("payments-context");
    let source_before = vec![
        git(source.path(), ["rev-parse", "HEAD"]),
        git(source.path(), ["status", "--porcelain=v1"]),
        git(
            source.path(),
            ["for-each-ref", "--format=%(refname):%(objectname)"],
        ),
        git(source.path(), ["config", "--local", "--list"]),
    ];

    let args = vec![
        OsString::from("devmap"),
        OsString::from("init"),
        OsString::from("--source"),
        source.path().as_os_str().to_owned(),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--goal"),
        OsString::from("Prevent duplicate payment capture"),
        OsString::from("--requirement"),
        OsString::from("docs/spec.md#payment-lock"),
    ];
    let output = run(args.clone()).unwrap();

    assert!(output.stdout.contains("draft_sha256="));
    assert!(output.stdout.contains("dirty_at_adoption=true"));
    assert!(output.stdout.contains("common-ground approve"));
    assert_eq!(git(&context, ["branch", "--show-current"]), "bootstrap/initial");
    assert!(context.join("bootstrap/common-ground-draft.json").is_file());
    assert!(!context.join("objects/common-ground").exists());
    assert!(
        !git_output(
            &context,
            ["show", "main:bootstrap/common-ground-draft.json"]
        )
        .status
        .success()
    );

    let draft = fs::read_to_string(context.join("bootstrap/common-ground-draft.json")).unwrap();
    assert!(draft.contains("Use a PostgreSQL advisory lock."));
    assert!(!draft.contains("Do something else."));
    assert!(!draft.contains("SECRET TEXT THAT MUST NOT BE CAPTURED"));
    assert!(draft.contains(r#""historical_scope":"not_reconstructed""#));

    let source_after = vec![
        git(source.path(), ["rev-parse", "HEAD"]),
        git(source.path(), ["status", "--porcelain=v1"]),
        git(
            source.path(),
            ["for-each-ref", "--format=%(refname):%(objectname)"],
        ),
        git(source.path(), ["config", "--local", "--list"]),
    ];
    assert_eq!(source_before, source_after);

    let first_head = git(&context, ["rev-parse", "HEAD"]);
    let second_output = run(args).unwrap();
    assert_eq!(first_head, git(&context, ["rev-parse", "HEAD"]));
    assert_eq!(output.stdout, second_output.stdout);
}

