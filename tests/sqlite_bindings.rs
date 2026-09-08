mod support;
use devmap::{
    dock::{DockService, ObservedTask, TaskLifecycle},
    git::SourceGitInspector,
    presence::PresenceStatus,
    store::RepositoryStore,
};
fn task(p: &std::path::Path) -> ObservedTask {
    ObservedTask {
        working_directory: None,
        subagents: None,
        lifecycle: TaskLifecycle::Present,
        session_id: "01a00000-0000-7000-8000-000000000001".into(),
        display_title: "task".into(),
        host: "local".into(),
        host_status: "active".into(),
        workspace_path: p.to_string_lossy().into(),
        status: PresenceStatus::Working,
        updated_at: "2026-09-03T10:00:00Z".into(),
    }
}
#[test]
fn sql_binding_watermark_advances_without_history_and_rejects_late() {
    let repo = support::committed_repo();
    let other = support::linked_worktree(repo.path(), "codex/move");
    let w = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let db = RepositoryStore::open(&w).unwrap();
    let c = rusqlite::Connection::open(db.path()).unwrap();
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let mut s = DockService::open(repo.path()).unwrap();
    s.replace_observed_tasks(vec![task(repo.path())], now)
        .unwrap();
    s.replace_observed_tasks(vec![task(repo.path())], now + time::Duration::seconds(10))
        .unwrap();
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM binding_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        1
    );
    assert_eq!(db.generation().unwrap(), 2);
    drop(s);
    let mut s = DockService::open(repo.path()).unwrap();
    s.replace_observed_tasks(vec![task(other.path())], now + time::Duration::seconds(5))
        .unwrap();
    assert_eq!(db.generation().unwrap(), 2);
    s.replace_observed_tasks(vec![task(other.path())], now + time::Duration::seconds(11))
        .unwrap();
    assert_eq!(db.generation().unwrap(), 3);
    assert!(!w.git_common_dir.join("devmap/task-bindings.jsonl").exists());
}
#[test]
fn binding_import_preserves_serialized_history_and_independent_watermark() {
    let repo = support::committed_repo();
    let other = support::linked_worktree(repo.path(), "codex/import");
    let w = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let now = time::OffsetDateTime::now_utc();
    let mut s = DockService::open(repo.path()).unwrap();
    s.replace_observed_tasks(vec![task(repo.path())], now)
        .unwrap();
    s.replace_observed_tasks(vec![task(other.path())], now + time::Duration::seconds(1))
        .unwrap();
    s.replace_observed_tasks(vec![task(other.path())], now + time::Duration::seconds(10))
        .unwrap();
    let before =
        serde_json::to_value(s.refresh(now + time::Duration::seconds(11)).unwrap()).unwrap();
    let db = RepositoryStore::open(&w).unwrap();
    let c = rusqlite::Connection::open(db.path()).unwrap();
    for line in std::fs::read_to_string(w.git_common_dir.join("devmap/task-bindings.jsonl"))
        .unwrap()
        .lines()
    {
        let r: devmap::journal::TaskBindingObservation = serde_json::from_str(line).unwrap();
        let json = serde_json::to_string(&r).unwrap();
        let id = devmap::canonical::sha256_hex(json.as_bytes());
        c.execute(
            "INSERT INTO binding_records VALUES(?1,?2,?3,?4,?5)",
            rusqlite::params![id, r.host, r.task_id, r.observed_at, json],
        )
        .unwrap();
    }
    let saved: serde_json::Value = serde_json::from_slice(
        &std::fs::read(w.git_common_dir.join("devmap/task-binding-watermarks.json")).unwrap(),
    )
    .unwrap();
    for row in saved["observations"].as_array().unwrap() {
        let key =
            serde_json::to_string(&(row[0].as_str().unwrap(), row[1].as_str().unwrap())).unwrap();
        c.execute(
            "INSERT INTO binding_watermarks VALUES(?1,?2,?3)",
            rusqlite::params![key, row[2].as_str().unwrap(), row.to_string()],
        )
        .unwrap();
    }
    c.execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let after =
        serde_json::to_value(s.refresh(now + time::Duration::seconds(11)).unwrap()).unwrap();
    assert_eq!(before["workspace_facts"], after["workspace_facts"]);
    s.replace_observed_tasks(vec![task(repo.path())], now + time::Duration::seconds(5))
        .unwrap();
    assert_eq!(db.generation().unwrap(), 0);
    c.execute_batch("CREATE TRIGGER reject_watermark BEFORE UPDATE ON binding_watermarks BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    s.replace_observed_tasks(vec![task(repo.path())], now + time::Duration::seconds(12))
        .unwrap();
    assert!(
        s.snapshot()
            .warnings
            .iter()
            .any(|warning| warning.code == "task_binding_history_unavailable")
    );
    assert!(
        s.snapshot()
            .workspace_facts
            .iter()
            .all(|facts| !facts.bindings_complete)
    );
    assert_eq!(db.generation().unwrap(), 0);
    assert_eq!(
        c.query_row("SELECT COUNT(*) FROM binding_records", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        2
    );
    c.execute_batch("DROP TRIGGER reject_watermark").unwrap();
    s.replace_observed_tasks(vec![task(repo.path())], now + time::Duration::seconds(12))
        .unwrap();
    assert_eq!(db.generation().unwrap(), 1);
}
