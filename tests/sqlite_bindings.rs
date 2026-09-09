mod support;
use devmap::{
    dock::{DockService, ObservedTask, TaskLifecycle},
    git::SourceGitInspector,
    presence::PresenceStatus,
    store::{RepositoryStore, migration},
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
    let history_path = w.git_common_dir.join("devmap/task-bindings.jsonl");
    let watermark_path = w.git_common_dir.join("devmap/task-binding-watermarks.json");
    let history_bytes = std::fs::read(&history_path).unwrap();
    let watermark_bytes = std::fs::read(&watermark_path).unwrap();
    let saved: serde_json::Value = serde_json::from_slice(&watermark_bytes).unwrap();
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("snapshot");
    migration::freeze(&w, &snapshot, now + time::Duration::seconds(11)).unwrap();
    migration::import_shadow(&w, &snapshot).unwrap();
    migration::activate(&w, &snapshot).unwrap();
    let db = RepositoryStore::open(&w).unwrap();
    let c = rusqlite::Connection::open(db.path()).unwrap();
    let imported = c
        .prepare("SELECT record_json FROM binding_records ORDER BY rowid")
        .unwrap()
        .query_map([], |r| r.get::<_, String>(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let original = std::str::from_utf8(&history_bytes)
        .unwrap()
        .lines()
        .map(|line| {
            let record: devmap::journal::TaskBindingObservation =
                serde_json::from_str(line).unwrap();
            serde_json::to_string(&record).unwrap()
        })
        .collect::<Vec<_>>();
    assert_eq!(imported, original);
    for row in saved["observations"].as_array().unwrap() {
        let key =
            serde_json::to_string(&(row[0].as_str().unwrap(), row[1].as_str().unwrap())).unwrap();
        let stored: (String, String, String) = c.query_row(
            "SELECT w.observed_at,w.record_json,c.observed_at FROM binding_watermarks w JOIN binding_origin_cursors c USING(source_scope) WHERE source_scope=?1",
            [key], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        ).unwrap();
        assert_eq!(
            stored,
            (
                row[2].as_str().unwrap().into(),
                row.to_string(),
                row[2].as_str().unwrap().into()
            )
        );
    }
    let sql_state = || {
        let tables = [
            "store_meta",
            "worktree_registry",
            "journal_sessions",
            "journal_records",
            "journal_heads",
            "presence_records",
            "presence_projection",
            "route_records",
            "binding_records",
            "binding_watermarks",
            "migration_sources",
            "route_origin_links",
            "binding_origin_links",
            "binding_origin_cursors",
        ];
        let actual = c
            .prepare(
                "SELECT name FROM sqlite_schema WHERE type='table' AND name NOT LIKE 'sqlite_%'",
            )
            .unwrap()
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .collect::<Result<std::collections::BTreeSet<_>, _>>()
            .unwrap();
        assert_eq!(actual, tables.iter().map(|name| name.to_string()).collect());
        tables
            .into_iter()
            .map(|table| {
                let mut statement = c
                    .prepare(&format!("SELECT * FROM {table} ORDER BY rowid"))
                    .unwrap();
                let columns = statement.column_count();
                let rows = statement
                    .query_map([], |row| {
                        (0..columns)
                            .map(|index| row.get::<_, rusqlite::types::Value>(index))
                            .collect::<Result<Vec<_>, _>>()
                    })
                    .unwrap()
                    .collect::<Result<Vec<_>, _>>()
                    .unwrap();
                (table, rows)
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let after =
        serde_json::to_value(s.refresh(now + time::Duration::seconds(11)).unwrap()).unwrap();
    assert_eq!(before["workspace_facts"], after["workspace_facts"]);
    let accepted_state = sql_state();
    s.replace_observed_tasks(vec![task(repo.path())], now + time::Duration::seconds(5))
        .unwrap();
    assert_eq!(db.generation().unwrap(), 0);
    assert_eq!(sql_state(), accepted_state);
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
    assert_eq!(sql_state(), accepted_state);
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
    assert_eq!(std::fs::read(history_path).unwrap(), history_bytes);
    assert_eq!(std::fs::read(watermark_path).unwrap(), watermark_bytes);
}
