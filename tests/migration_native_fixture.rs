//! Explicit disposable fixture export, excluded from normal test runs.
use devmap::{
    dock::{ObservedTask, TaskLifecycle},
    git::SourceGitInspector,
    presence::PresenceStatus,
    store::migration,
};
use std::{fs, path::PathBuf};
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

#[test]
#[ignore = "writes a late legacy event only in an already migrated disposable native fixture"]
fn native_old_writer_is_detected_without_losing_either_store() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/verification")
        .canonicalize()
        .unwrap();
    let manifest_path = PathBuf::from(std::env::var("DEVMAP_NATIVE_MANIFEST").unwrap())
        .canonicalize()
        .unwrap();
    assert!(manifest_path.starts_with(&allowed));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["scope"], "legacy_native_process_fixture");
    let source = PathBuf::from(manifest["source"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    assert!(source.starts_with(&allowed));
    let workspace = SourceGitInspector::open(&source)
        .unwrap()
        .workspace()
        .unwrap();
    assert!(
        workspace
            .git_common_dir
            .canonicalize()
            .unwrap()
            .starts_with(&allowed)
    );
    let exe = PathBuf::from(std::env::var("DEVMAP_BASELINE_EXE").unwrap());
    assert_eq!(
        devmap::canonical::sha256_hex(&fs::read(&exe).unwrap()).to_uppercase(),
        "A1CFBB1C46BD9026B18DB67DA9F70485B0BB20C6C54FD779475B52531731D419"
    );
    assert!(migration::verify(&workspace).unwrap().verified);
    let db = rusqlite::Connection::open_with_flags(
        workspace.git_common_dir.join("devmap/devmap.db"),
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
    .unwrap();
    let sql_state = || -> (String, i64, i64, String) {
        db.query_row("SELECT backend_state,generation,(SELECT count(*) FROM journal_records),(SELECT group_concat(record_json,'') FROM journal_records) FROM store_meta", [], |r| Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?))).unwrap()
    };
    let before = sql_state();
    let session = manifest["inventory"][0]["id"].as_str().unwrap();
    let legacy = workspace
        .git_dir
        .join("devmap/sessions")
        .join(session)
        .join("events.ndjson");
    let legacy_before = fs::read(&legacy).unwrap();
    let frozen_path = manifest_path
        .parent()
        .unwrap()
        .join("sqlite-frozen/0/sessions")
        .join(session)
        .join("events.ndjson");
    let frozen_before = fs::read(&frozen_path).unwrap();
    let head = Command::new("git")
        .arg("-C")
        .arg(&source)
        .args(["rev-parse", "HEAD"])
        .output()
        .unwrap();
    assert!(head.status.success());
    let request = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"devmap_record_evidence","arguments":{
        "session_id":session,"agent_id":"fixture-agent-0","event_id":"fixture-late-old-writer","occurred_at":"2026-09-08T19:00:00Z",
        "kind":"test","target":format!("commit:{}",String::from_utf8(head.stdout).unwrap().trim()),"outcome":"pending"}}});
    let mut child = Command::new(&exe)
        .args(["mcp", "--source"])
        .arg(&source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    writeln!(input,"{}",serde_json::json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"legacy-drift-fixture","version":"1"}}})).unwrap();
    writeln!(input, "{request}").unwrap();
    drop(input);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if child.try_wait().unwrap().is_some() {
            break;
        }
        if std::time::Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("owned legacy fixture process exceeded 30 seconds");
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let response = child.wait_with_output().unwrap();
    assert!(response.status.success());
    let values = String::from_utf8(response.stdout)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str::<serde_json::Value>(l).unwrap())
        .collect::<Vec<_>>();
    let answer = values.iter().find(|v| v["id"] == 1).unwrap();
    assert!(
        answer.get("error").is_none() && answer["result"]["isError"] != true,
        "{answer}"
    );
    let legacy_after = fs::read(&legacy).unwrap();
    assert!(legacy_after.starts_with(&legacy_before) && legacy_after.len() > legacy_before.len());
    let error = migration::verify(&workspace)
        .expect_err("late old writer must be diagnosed")
        .to_string();
    assert!(error.contains("drift"), "{error}");
    assert_eq!(sql_state(), before);
    assert_eq!(fs::read(&legacy).unwrap(), legacy_after);
    assert_eq!(fs::read(&frozen_path).unwrap(), frozen_before);
    fs::write(manifest_path.parent().unwrap().join("late-writer-report.json"), serde_json::to_vec_pretty(&serde_json::json!({
        "scope":"frozen_native_binary_late_write", "response":answer,"verification_error":error,
        "sql_unchanged":true,"legacy_appended_bytes":legacy_after.len()-legacy_before.len(),"snapshot_unchanged":true
    })).unwrap()).unwrap();
}

#[test]
#[ignore = "requires a native legacy fixture under this checkout target/verification"]
fn export_native_migration_pair() {
    let allowed = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target/verification")
        .canonicalize()
        .unwrap();
    let manifest_path = PathBuf::from(std::env::var("DEVMAP_NATIVE_MANIFEST").unwrap())
        .canonicalize()
        .unwrap();
    assert!(manifest_path.starts_with(&allowed));
    let manifest: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest_path).unwrap()).unwrap();
    assert_eq!(manifest["scope"], "legacy_native_process_fixture");
    let source = PathBuf::from(manifest["source"].as_str().unwrap())
        .canonicalize()
        .unwrap();
    assert!(source.starts_with(&allowed));
    let workspace = SourceGitInspector::open(&source)
        .unwrap()
        .workspace()
        .unwrap();
    assert!(
        workspace
            .git_common_dir
            .canonicalize()
            .unwrap()
            .starts_with(&allowed)
    );
    let output = manifest_path.parent().unwrap();
    let snapshot = output.join("sqlite-frozen");
    let old: serde_json::Value =
        serde_json::from_slice(&fs::read(output.join("baseline-snapshot.json")).unwrap()).unwrap();
    let now = OffsetDateTime::parse(old["generated_at"].as_str().unwrap(), &Rfc3339).unwrap();
    let tasks = manifest["inventory"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| ObservedTask {
            working_directory: None,
            subagents: None,
            lifecycle: TaskLifecycle::Present,
            session_id: t["id"].as_str().unwrap().into(),
            display_title: t["title"].as_str().unwrap().into(),
            host: t["hostId"].as_str().unwrap().into(),
            host_status: t["status"].as_str().unwrap().into(),
            workspace_path: t["cwd"].as_str().unwrap().into(),
            status: if t["status"] == "waiting" {
                PresenceStatus::Waiting
            } else {
                PresenceStatus::Working
            },
            // Match the production MCP parser's millisecond-to-second contract.
            updated_at: OffsetDateTime::from_unix_timestamp(
                t["updatedAt"].as_i64().unwrap() / 1_000,
            )
            .unwrap()
            .format(&Rfc3339)
            .unwrap(),
        })
        .collect::<Vec<_>>();
    migration::freeze(&workspace, &snapshot, now).unwrap();
    migration::import_shadow(&workspace, &snapshot).unwrap();
    let pair = migration::compare_snapshot_with_inventory(
        &workspace,
        &snapshot,
        &tasks,
        old["task_inventory_synced_at"].as_str().map(str::to_owned),
        old["task_observation"]["complete"].as_bool().unwrap(),
    )
    .unwrap();
    assert_eq!(pair.legacy, pair.sql);
    // The original real process performed multiple refreshes. Only its two
    // process-local transport counters differ from an initial projection.
    // Actual owner-restart counter monotonicity is a separate runtime gate.
    let mut original = old.clone();
    original["revision"] = serde_json::json!(1);
    original["observation_revision"] = serde_json::json!(1);
    assert_eq!(original, serde_json::to_value(&pair.legacy).unwrap());
    for (name, model) in [
        ("migration-legacy.json", &pair.legacy),
        ("migration-sql.json", &pair.sql),
    ] {
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(output.join(name))
            .unwrap();
        serde_json::to_writer_pretty(&mut file, model).unwrap();
    }
    migration::activate(&workspace, &snapshot).unwrap();
    assert!(migration::verify(&workspace).unwrap().verified);
    println!(
        "Native legacy fixture migrated; frozen full-model pair at {}",
        output.display()
    );
}
