mod support;

use std::ffi::OsString;
use std::fs;
use std::process::Command;

use devmap::run;
use support::{committed_repo, git};

#[test]
fn existing_project_adopts_an_explicit_boundary_without_historical_backfill() {
    let source = committed_repo();
    fs::write(
        source.path().join("service.rs"),
        "fn version() -> u8 { 1 }\n",
    )
    .unwrap();
    git(source.path(), ["add", "--", "service.rs"]);
    git(source.path(), ["commit", "-m", "add service"]);
    fs::write(
        source.path().join("service.rs"),
        "fn version() -> u8 { 2 }\n",
    )
    .unwrap();
    git(source.path(), ["add", "--", "service.rs"]);
    git(source.path(), ["commit", "-m", "change implementation"]);
    fs::create_dir(source.path().join("docs")).unwrap();
    fs::write(
        source.path().join("docs/adoption.md"),
        "# DevMap Adoption\nFrom the current main commit, preserve an evidence-backed development route.\n",
    )
    .unwrap();
    git(source.path(), ["add", "--", "docs/adoption.md"]);
    git(source.path(), ["commit", "-m", "define adoption point"]);
    fs::write(source.path().join("unfinished.rs"), "// local work\n").unwrap();

    let parent = tempfile::tempdir().unwrap();
    let context = parent.path().join("service-context");
    let boundary = git(source.path(), ["rev-parse", "HEAD"]);
    let source_snapshot = vec![
        boundary.clone(),
        git(source.path(), ["status", "--porcelain=v1"]),
        git(
            source.path(),
            ["for-each-ref", "--format=%(refname):%(objectname)"],
        ),
        git(source.path(), ["config", "--local", "--list"]),
    ];
    let init_args = vec![
        OsString::from("devmap"),
        OsString::from("init"),
        OsString::from("--source"),
        source.path().as_os_str().to_owned(),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--goal"),
        OsString::from("Adopt DevMap without inventing prior decisions"),
        OsString::from("--requirement"),
        OsString::from("docs/adoption.md#devmap-adoption"),
    ];

    run(init_args.clone()).unwrap();
    let draft = fs::read_to_string(context.join("bootstrap/common-ground-draft.json")).unwrap();
    assert!(draft.contains(r#""historical_scope":"not_reconstructed""#));
    assert!(!draft.contains("agent_decision"));
    assert!(!draft.contains("alternatives"));
    assert!(draft.contains(&boundary));
    assert_eq!(
        source_snapshot,
        vec![
            git(source.path(), ["rev-parse", "HEAD"]),
            git(source.path(), ["status", "--porcelain=v1"]),
            git(
                source.path(),
                ["for-each-ref", "--format=%(refname):%(objectname)"],
            ),
            git(source.path(), ["config", "--local", "--list"]),
        ]
    );

    run(vec![
        OsString::from("devmap"),
        OsString::from("common-ground"),
        OsString::from("approve"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--actor"),
        OsString::from("Dylan"),
    ])
    .unwrap();
    let report = run(vec![
        OsString::from("devmap"),
        OsString::from("status"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--json"),
    ])
    .unwrap();
    assert_eq!(report.exit_code, 0);
    assert!(report.stdout.contains(r#""valid":true"#));

    let clone = parent.path().join("context-clone");
    let clone_output = Command::new("git")
        .args(["clone", "--quiet"])
        .arg(&context)
        .arg(&clone)
        .output()
        .unwrap();
    assert!(clone_output.status.success());
    let cloned_report = run(vec![
        OsString::from("devmap"),
        OsString::from("status"),
        OsString::from("--context"),
        clone.as_os_str().to_owned(),
    ])
    .unwrap();
    assert_eq!(cloned_report.exit_code, 0);
    assert!(cloned_report.stdout.contains("integrity=valid"));

    let error = run(init_args).unwrap_err();
    assert!(error.to_string().contains("already approved"));
    assert!(
        !git(&context, ["branch", "--list", "bootstrap/initial"]).contains("bootstrap/initial")
    );
}
