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
    assert_eq!(
        git(&context, ["branch", "--show-current"]),
        "bootstrap/initial"
    );
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

    let approval = run(vec![
        OsString::from("devmap"),
        OsString::from("common-ground"),
        OsString::from("approve"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--actor"),
        OsString::from("Dylan"),
    ])
    .unwrap();

    assert!(
        approval
            .stdout
            .contains("common_ground_id=common-ground:sha256-")
    );
    assert!(approval.stdout.contains("approval_id=approval:sha256-"));
    assert!(approval.stdout.contains("capture_grade=C"));
    assert_eq!(git(&context, ["branch", "--show-current"]), "main");
    assert!(!context.join("bootstrap/common-ground-draft.json").exists());
    assert!(context.join("objects/common-ground").is_dir());
    assert!(context.join("objects/approval").is_dir());
    assert!(context.join("manifests/common-ground.json").is_file());
    assert!(context.join("state/current.json").is_file());
    assert!(
        !git(&context, ["branch", "--list", "bootstrap/initial"]).contains("bootstrap/initial")
    );

    let refs = git(&context, ["for-each-ref", "--format=%(refname)"]);
    assert!(!refs.contains("refs/devmap"));
    assert!(!refs.contains("refs/notes"));
}

#[test]
fn approval_refuses_a_tampered_draft() {
    let source = committed_repo();
    let parent = tempfile::tempdir().unwrap();
    let context = parent.path().join("context");
    run(vec![
        OsString::from("devmap"),
        OsString::from("init"),
        OsString::from("--source"),
        source.path().as_os_str().to_owned(),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--goal"),
        OsString::from("Adopt DevMap from this commit"),
    ])
    .unwrap();
    fs::write(
        context.join("bootstrap/common-ground-draft.json"),
        br#"{"tampered":true}"#,
    )
    .unwrap();

    let error = run(vec![
        OsString::from("devmap"),
        OsString::from("common-ground"),
        OsString::from("approve"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--actor"),
        OsString::from("Dylan"),
    ])
    .unwrap_err();

    assert!(error.to_string().contains("clean"));
    assert!(!context.join("objects/common-ground").exists());
}

#[test]
fn approval_requires_a_human_actor_and_the_bootstrap_branch() {
    let source = committed_repo();
    let parent = tempfile::tempdir().unwrap();
    let context = parent.path().join("context");
    run(vec![
        OsString::from("devmap"),
        OsString::from("init"),
        OsString::from("--source"),
        source.path().as_os_str().to_owned(),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--goal"),
        OsString::from("Adopt DevMap from this commit"),
    ])
    .unwrap();

    let blank_actor = run(vec![
        OsString::from("devmap"),
        OsString::from("common-ground"),
        OsString::from("approve"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--actor"),
        OsString::from("  "),
    ])
    .unwrap_err();
    assert!(blank_actor.to_string().contains("actor"));
    assert!(!context.join("objects/common-ground").exists());

    git(&context, ["checkout", "main"]);
    let wrong_branch = run(vec![
        OsString::from("devmap"),
        OsString::from("common-ground"),
        OsString::from("approve"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--actor"),
        OsString::from("Dylan"),
    ])
    .unwrap_err();
    assert!(wrong_branch.to_string().contains("branch"));
}
