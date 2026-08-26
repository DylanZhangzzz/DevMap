mod support;

use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use devmap::run;
use support::{committed_repo, git};

fn initialize(context: &Path, approve: bool) -> (tempfile::TempDir, String) {
    let source = committed_repo();
    let boundary = git(source.path(), ["rev-parse", "HEAD"]);
    run(vec![
        OsString::from("devmap"),
        OsString::from("init"),
        OsString::from("--source"),
        source.path().as_os_str().to_owned(),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--goal"),
        OsString::from("Establish the first shared development map"),
    ])
    .unwrap();
    if approve {
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
    }
    (source, boundary)
}

fn status_args(context: &Path, json: bool) -> Vec<OsString> {
    let mut args = vec![
        OsString::from("devmap"),
        OsString::from("status"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
    ];
    if json {
        args.push(OsString::from("--json"));
    }
    args
}

#[test]
fn status_reports_draft_and_approved_lifecycles() {
    let parent = tempfile::tempdir().unwrap();
    let draft_context = parent.path().join("draft-context");
    let (_source, boundary) = initialize(&draft_context, false);

    let draft = run(status_args(&draft_context, false)).unwrap();
    assert_eq!(draft.exit_code, 0);
    assert!(draft.stdout.contains("lifecycle=draft"));
    assert!(
        draft
            .stdout
            .contains(&format!("adoption_boundary_commit={boundary}"))
    );
    assert!(draft.stdout.contains("capture_grade=C"));
    assert!(draft.stdout.contains("integrity=valid"));

    let approved_context = parent.path().join("approved-context");
    let (_source, approved_boundary) = initialize(&approved_context, true);
    let approved = run(status_args(&approved_context, true)).unwrap();
    assert_eq!(approved.exit_code, 0);
    let json: serde_json::Value = serde_json::from_str(approved.stdout.trim()).unwrap();
    assert_eq!(json["lifecycle"], "approved");
    assert_eq!(json["adoption_boundary_commit"], approved_boundary);
    assert_eq!(json["capture_grade"], "C");
    assert_eq!(json["integrity"]["valid"], true);
    assert_eq!(json["object_counts"]["common-ground"], 1);
    assert_eq!(json["object_counts"]["approval"], 1);
}

#[test]
fn status_detects_tampered_and_missing_canonical_objects() {
    let parent = tempfile::tempdir().unwrap();
    let context = parent.path().join("context");
    initialize(&context, true);
    let object = only_json_file(context.join("objects/common-ground"));
    let original = fs::read(&object).unwrap();
    fs::write(&object, br#"{"tampered":true}"#).unwrap();

    let tampered = run(status_args(&context, false)).unwrap();
    assert_eq!(tampered.exit_code, 1);
    assert!(tampered.stdout.contains("integrity=invalid"));
    assert!(tampered.stdout.contains("hash_mismatch"));

    let process = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args(["status", "--context"])
        .arg(&context)
        .output()
        .unwrap();
    assert!(!process.status.success());
    assert!(
        String::from_utf8(process.stdout)
            .unwrap()
            .contains("integrity=invalid")
    );

    fs::write(&object, original).unwrap();
    let approval = only_json_file(context.join("objects/approval"));
    fs::remove_file(approval).unwrap();
    let missing = run(status_args(&context, false)).unwrap();
    assert_eq!(missing.exit_code, 1);
    assert!(missing.stdout.contains("missing_object"));
}

#[test]
fn status_rejects_custom_devmap_refs() {
    let parent = tempfile::tempdir().unwrap();
    let context = parent.path().join("context");
    initialize(&context, true);
    git(&context, ["update-ref", "refs/devmap/forbidden", "HEAD"]);

    let report = run(status_args(&context, false)).unwrap();

    assert_eq!(report.exit_code, 1);
    assert!(report.stdout.contains("forbidden_ref"));
}

fn only_json_file(directory: PathBuf) -> PathBuf {
    let files: Vec<_> = fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().and_then(|value| value.to_str()) == Some("json"))
        .collect();
    assert_eq!(files.len(), 1);
    files.into_iter().next().unwrap()
}
