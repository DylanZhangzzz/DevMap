mod support;

use std::ffi::OsString;
use std::fs;
use std::io::Cursor;
use std::path::Path;

use devmap::adapter::{install_adapter, plan_adapter, verify_adapter};
use devmap::cli::{AdapterHost, HookHandleArgs};
use devmap::events::CaptureGrade;
use devmap::git::SourceGitInspector;
use devmap::hook::handle_hook;
use devmap::journal::JournalStore;
use devmap::run;
use support::{assert_only_source_paths_changed, committed_repo, source_snapshot};

#[test]
fn adapter_installation_changes_only_the_named_file_and_no_git_state() {
    for (host, relative) in [
        (AdapterHost::Codex, Path::new(".codex/hooks.json")),
        (AdapterHost::Claude, Path::new(".claude/settings.json")),
        (AdapterHost::GenericMcp, Path::new(".devmap/mcp.json")),
    ] {
        let repository = committed_repo();
        let before = source_snapshot(repository.path());

        let plan = plan_adapter(repository.path(), host).unwrap();
        assert_eq!(plan.config_path, repository.path().join(relative));
        assert_eq!(
            source_snapshot(repository.path()),
            before,
            "plan must be read-only"
        );
        let token = plan.plan_digest.clone();
        install_adapter(plan, &token).unwrap();
        let report = verify_adapter(repository.path(), host).unwrap();
        assert_eq!(report.capture_grade, CaptureGrade::D);
        assert!(report.configured);
        assert!(report.missing.is_empty());
        assert!(report.modified.is_empty());

        let after = source_snapshot(repository.path());
        assert_only_source_paths_changed(&before, &after, &[relative]);
        assert_eq!(
            support::git(repository.path(), ["diff", "--cached", "--name-only"]),
            "",
            "adapter installation must not stage its config"
        );
    }
}

#[test]
fn malformed_hook_and_adapter_inputs_fail_closed() {
    let hook_repository = committed_repo();
    let workspace = SourceGitInspector::open(hook_repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let before_hook = source_snapshot(hook_repository.path());
    let mut malformed = Cursor::new(br#"{"session_id": "broken""#);
    assert!(
        handle_hook(
            HookHandleArgs {
                source: hook_repository.path().to_path_buf(),
                host: AdapterHost::Codex,
                event: "SessionStart".into(),
            },
            &mut malformed,
        )
        .is_err()
    );
    assert!(
        JournalStore::open(&workspace, "broken")
            .unwrap()
            .replay()
            .unwrap()
            .is_empty()
    );
    assert_eq!(source_snapshot(hook_repository.path()), before_hook);

    for (host, relative) in [
        (AdapterHost::Codex, Path::new(".codex/hooks.json")),
        (AdapterHost::Claude, Path::new(".claude/settings.json")),
        (AdapterHost::GenericMcp, Path::new(".devmap/mcp.json")),
    ] {
        let repository = committed_repo();
        let path = repository.path().join(relative);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"{not valid json").unwrap();
        let before = source_snapshot(repository.path());
        let failed = plan_adapter(repository.path(), host).is_err();
        assert!(failed);
        assert_eq!(fs::read(&path).unwrap(), b"{not valid json");
        assert_eq!(source_snapshot(repository.path()), before);
    }
}

#[test]
fn verification_reports_installed_capability_not_requested_grade() {
    let repository = committed_repo();
    let missing = verify_adapter(repository.path(), AdapterHost::Codex).unwrap();
    assert_eq!(missing.capture_grade, CaptureGrade::D);
    assert!(!missing.missing.is_empty());

    let plan = plan_adapter(repository.path(), AdapterHost::Codex).unwrap();
    let token = plan.plan_digest.clone();
    install_adapter(plan, &token).unwrap();
    let installed = verify_adapter(repository.path(), AdapterHost::Codex).unwrap();
    assert_eq!(installed.capture_grade, CaptureGrade::D);
    assert!(installed.configured);
    assert!(!installed.activation_verified);
    assert!(!installed.activation_reasons.is_empty());

    let config = repository.path().join(".codex/hooks.json");
    let mut document: serde_json::Value =
        serde_json::from_slice(&fs::read(&config).unwrap()).unwrap();
    document["hooks"]["SessionStart"][0]["hooks"][0]["command"] = serde_json::json!(
        "devmap hook handle --host codex --event SessionStart --binding-id devmap/v1/codex/SessionStart --tampered"
    );
    fs::write(&config, serde_json::to_vec_pretty(&document).unwrap()).unwrap();
    let drifted = verify_adapter(repository.path(), AdapterHost::Codex).unwrap();
    assert_eq!(drifted.capture_grade, CaptureGrade::D);
    assert!(!drifted.modified.is_empty());
    assert!(!drifted.drift_reasons.is_empty());
}

#[test]
fn phase_1a_adoption_and_integrity_flow_remains_available() {
    let source = committed_repo();
    fs::create_dir(source.path().join("docs")).unwrap();
    fs::write(
        source.path().join("docs/adoption.md"),
        "# Adoption\nPreserve the verified route from this boundary.\n",
    )
    .unwrap();
    support::git(source.path(), ["add", "--", "docs/adoption.md"]);
    support::git(source.path(), ["commit", "-m", "add adoption requirement"]);
    let source_before = source_snapshot(source.path());
    let parent = tempfile::tempdir().unwrap();
    let context = parent.path().join("context");

    let init = run(vec![
        OsString::from("devmap"),
        OsString::from("init"),
        OsString::from("--source"),
        source.path().as_os_str().to_owned(),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--goal"),
        OsString::from("Adopt the current source boundary"),
        OsString::from("--requirement"),
        OsString::from("docs/adoption.md#adoption"),
    ])
    .unwrap();
    assert_eq!(init.exit_code, 0);
    run(vec![
        OsString::from("devmap"),
        OsString::from("common-ground"),
        OsString::from("approve"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--actor"),
        OsString::from("Acceptance Reviewer"),
    ])
    .unwrap();
    let status = run(vec![
        OsString::from("devmap"),
        OsString::from("status"),
        OsString::from("--context"),
        context.as_os_str().to_owned(),
        OsString::from("--json"),
    ])
    .unwrap();
    assert_eq!(status.exit_code, 0);
    assert!(status.stdout.contains(r#""valid":true"#));
    assert_eq!(source_snapshot(source.path()), source_before);
    assert!(!source.path().join("work").exists());
}
