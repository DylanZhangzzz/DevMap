mod support;
use devmap::{
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::SourceWorkspace,
    journal::JournalStore,
    presence::{PresenceSignal, PresenceStore},
    store::migration,
};
use devmap::{git::SourceGitInspector, store::RepositoryStore};
use support::committed_repo;
use time::OffsetDateTime;
fn now() -> OffsetDateTime {
    OffsetDateTime::from_unix_timestamp(1788861600).unwrap()
}
fn event(session: &str, id: &str, sequence: u64) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        id,
        EventType::CaptureGap,
        sequence,
        "2026-09-08T10:00:00Z",
        HostIdentity::new("test", "1").unwrap(),
        ActorIdentity::new("a", None).unwrap(),
        SessionContext::new(session, None, "fixture", None, None, None).unwrap(),
        serde_json::json!({"reason":"fixture"}),
    )
    .unwrap()
}
fn workspace(path: &std::path::Path) -> SourceWorkspace {
    SourceGitInspector::open(path).unwrap().workspace().unwrap()
}
fn append(w: &SourceWorkspace, session: &str, id: &str) -> devmap::journal::JournalRecord {
    JournalStore::open(w, session)
        .unwrap()
        .append(event(session, id, 1))
        .unwrap()
}
fn database(w: &SourceWorkspace) -> rusqlite::Connection {
    rusqlite::Connection::open(RepositoryStore::open_existing(w).unwrap().unwrap().path()).unwrap()
}

#[test]
fn inspect_is_read_only_and_empty_migration_activates_durably() {
    let repo = committed_repo();
    let backup = tempfile::tempdir().unwrap();
    let source = repo.path().to_str().unwrap();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let inspected = devmap::run(["devmap", "storage", "inspect", "--source", source]).unwrap();
    assert!(inspected.stdout.contains("legacy"));
    assert!(
        RepositoryStore::open_existing(&workspace)
            .unwrap()
            .is_none()
    );
    let destination = backup.path().join("frozen");
    let args = [
        "devmap",
        "storage",
        "migrate",
        "--source",
        source,
        "--backup-dir",
        destination.to_str().unwrap(),
    ];
    let migrated = devmap::run(args).unwrap();
    assert!(migrated.stdout.contains("active"));
    assert!(destination.join("manifest.json").exists());
    let repeated = devmap::run(args).unwrap();
    assert!(repeated.stdout.contains("active"));
    assert!(devmap::run(["devmap", "storage", "verify", "--source", source]).is_ok());
}

#[test]
fn unknown_artifact_refuses_activation_and_preserves_bytes() {
    let repo = committed_repo();
    let backup = tempfile::tempdir().unwrap();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let root = workspace.git_common_dir.join("devmap");
    std::fs::create_dir(&root).unwrap();
    let path = root.join("unrecognized.bin");
    std::fs::write(&path, b"retain me").unwrap();
    let destination = backup.path().join("frozen");
    let result = devmap::run([
        "devmap",
        "storage",
        "migrate",
        "--source",
        repo.path().to_str().unwrap(),
        "--backup-dir",
        destination.to_str().unwrap(),
    ]);
    assert!(result.is_err());
    assert_eq!(std::fs::read(path).unwrap(), b"retain me");
    if let Some(store) = RepositoryStore::open_existing(&workspace).unwrap() {
        let c = rusqlite::Connection::open(store.path()).unwrap();
        assert_eq!(
            c.query_row("SELECT backend_state FROM store_meta", [], |r| r
                .get::<_, String>(0))
                .unwrap(),
            "shadow"
        );
    }
}

#[test]
fn mixed_worktree_history_presence_and_empty_missing_journals_are_preserved() {
    let repo = committed_repo();
    let linked = support::linked_worktree(repo.path(), "linked");
    let w = workspace(repo.path());
    let lw = workspace(linked.path());
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    let main = append(&w, "main", "a");
    let child = append(&lw, "linked", "b");
    let p = PresenceStore::open(&w).unwrap();
    p.observe(
        PresenceSignal::AcceptedRecords(std::slice::from_ref(&main)),
        now(),
    )
    .unwrap();
    let cp = PresenceStore::open(&lw).unwrap();
    cp.observe(
        PresenceSignal::AcceptedRecords(std::slice::from_ref(&child)),
        now(),
    )
    .unwrap();
    let waiting = cp
        .observe(
            PresenceSignal::ExplicitWaiting {
                session_id: "linked",
                activity_id: Some("wait"),
            },
            now(),
        )
        .unwrap();
    std::fs::create_dir_all(w.git_dir.join("devmap/sessions/empty")).unwrap();
    std::fs::write(w.git_dir.join("devmap/sessions/empty/events.ndjson"), b"").unwrap();
    std::fs::create_dir_all(w.git_dir.join("devmap/sessions/missing")).unwrap();
    let manifest = migration::freeze(&w, &snapshot, now()).unwrap();
    assert_eq!(manifest.origins.len(), 2);
    assert!(manifest.files.iter().any(|f| f.record_count == 1));
    assert!(
        manifest
            .origins
            .iter()
            .all(|o| !o.git_dir.starts_with(&snapshot))
    );
    migration::import_shadow(&w, &snapshot).unwrap();
    let pair = migration::compare_snapshot(&w, &snapshot, &[]).unwrap();
    assert_eq!(
        serde_json::to_value(&pair.legacy).unwrap(),
        serde_json::to_value(&pair.sql).unwrap()
    );
    migration::activate(&w, &snapshot).unwrap();
    assert_eq!(
        JournalStore::open(&w, "main").unwrap().replay().unwrap(),
        vec![main]
    );
    assert_eq!(
        JournalStore::open(&lw, "linked").unwrap().replay().unwrap(),
        vec![child]
    );
    assert!(
        PresenceStore::open(&lw)
            .unwrap()
            .load_all()
            .records
            .contains(&waiting)
    );
    let c = database(&w);
    assert_eq!(
        c.query_row("SELECT count(*) FROM journal_heads", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        3
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM journal_sessions WHERE session_id='missing'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM presence_projection WHERE baseline_source='legacy_import'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        2
    );
    for f in manifest.files {
        let original = std::fs::read(
            manifest.origins[f.origin]
                .git_dir
                .join("devmap")
                .join(&f.relative),
        )
        .unwrap();
        assert_eq!(devmap::canonical::sha256_hex(&original), f.sha256);
    }
}

#[test]
fn route_revisions_retry_and_newer_binding_watermark_survive() {
    use devmap::route_plan::{PlanInput, RoutePlanStore};
    let repo = committed_repo();
    let w = workspace(repo.path());
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    let worktree = devmap::worktrees::WorktreeScanner::scan(&w).unwrap()[0]
        .worktree_id
        .clone();
    let plans = RoutePlanStore::open(&w).unwrap();
    let mut input = PlanInput {
        delivery: Default::default(),
        request_id: "a".into(),
        route_id: None,
        expected_revision: 0,
        worktree_id: worktree.clone(),
        goal: "goal".into(),
        target_ref: None,
        milestones: vec![],
        source: "user".into(),
        abandoned: false,
    };
    let first = plans.set(input.clone()).unwrap();
    input.request_id = "b".into();
    input.route_id = Some(first.route_id.clone());
    input.expected_revision = 1;
    input.goal = "next".into();
    let second = plans.set(input.clone()).unwrap();
    let task = devmap::dock::ObservedTask {
        working_directory: None,
        subagents: None,
        lifecycle: devmap::dock::TaskLifecycle::Present,
        session_id: "01a00000-0000-7000-8000-000000000001".into(),
        display_title: "task".into(),
        host: "local".into(),
        host_status: "active".into(),
        workspace_path: repo.path().to_string_lossy().into(),
        status: devmap::presence::PresenceStatus::Working,
        updated_at: "2026-09-08T10:00:00Z".into(),
    };
    let mut dock = devmap::dock::DockService::open(repo.path()).unwrap();
    dock.replace_observed_tasks(vec![task.clone()], now())
        .unwrap();
    dock.replace_observed_tasks(vec![task], now() + time::Duration::seconds(10))
        .unwrap();
    drop(dock);
    migration::freeze(&w, &snapshot, now()).unwrap();
    migration::import_shadow(&w, &snapshot).unwrap();
    migration::activate(&w, &snapshot).unwrap();
    assert_eq!(plans.list().unwrap(), vec![second.clone()]);
    assert_eq!(plans.set(input).unwrap(), second);
    assert_eq!(first.start_commit, second.start_commit);
    assert_eq!(
        database(&w)
            .query_row("SELECT count(*) FROM route_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(
        database(&w)
            .query_row("SELECT count(*) FROM binding_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        1
    );
    let saved: String = database(&w)
        .query_row("SELECT observed_at FROM binding_watermarks", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert_eq!(
        OffsetDateTime::parse(&saved, &time::format_description::well_known::Rfc3339).unwrap(),
        now() + time::Duration::seconds(10)
    );
}

#[test]
fn truncated_pending_and_unknown_session_input_are_rejected_without_repair() {
    for artifact in ["events.intent", "events.index.tmp", "unknown"] {
        let repo = committed_repo();
        let w = workspace(repo.path());
        append(&w, "s", "a");
        let path = w.git_dir.join("devmap/sessions/s").join(artifact);
        std::fs::write(&path, b"pending").unwrap();
        let backup = tempfile::tempdir().unwrap();
        assert!(migration::freeze(&w, &backup.path().join("frozen"), now()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"pending");
    }
    let repo = committed_repo();
    let w = workspace(repo.path());
    append(&w, "s", "a");
    let path = w.git_dir.join("devmap/sessions/s/events.ndjson");
    let mut bytes = std::fs::read(&path).unwrap();
    bytes.pop();
    std::fs::write(&path, &bytes).unwrap();
    let backup = tempfile::tempdir().unwrap();
    assert!(migration::freeze(&w, &backup.path().join("frozen"), now()).is_err());
    assert_eq!(std::fs::read(path).unwrap(), bytes);
}

#[test]
fn drift_between_stages_blocks_import_and_activation() {
    let repo = committed_repo();
    let w = workspace(repo.path());
    append(&w, "s", "a");
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    migration::freeze(&w, &snapshot, now()).unwrap();
    JournalStore::open(&w, "s")
        .unwrap()
        .append(event("s", "b", 2))
        .unwrap();
    assert!(migration::import_shadow(&w, &snapshot).is_err());
    assert_eq!(
        database(&w)
            .query_row("SELECT count(*) FROM journal_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        0
    );
    let second = backup.path().join("second");
    migration::freeze(&w, &second, now()).unwrap();
    migration::import_shadow(&w, &second).unwrap();
    JournalStore::open(&w, "s")
        .unwrap()
        .append(event("s", "c", 3))
        .unwrap();
    assert!(migration::activate(&w, &second).is_err());
    assert_eq!(migration::inspect(&w).unwrap().backend, "shadow");
}

#[test]
fn failed_import_and_interrupted_activation_roll_back_then_retry() {
    let repo = committed_repo();
    let w = workspace(repo.path());
    append(&w, "s", "a");
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    migration::freeze(&w, &snapshot, now()).unwrap();
    let c = database(&w);
    c.execute_batch("CREATE TRIGGER fail_import BEFORE INSERT ON journal_records BEGIN SELECT RAISE(ABORT,'injected crash'); END;").unwrap();
    assert!(migration::import_shadow(&w, &snapshot).is_err());
    assert_eq!(
        c.query_row("SELECT count(*) FROM worktree_registry", [], |r| r
            .get::<_, i64>(0))
            .unwrap(),
        0
    );
    c.execute_batch("DROP TRIGGER fail_import").unwrap();
    migration::import_shadow(&w, &snapshot).unwrap();
    c.execute_batch("CREATE TRIGGER fail_activation BEFORE UPDATE OF backend_state ON store_meta BEGIN SELECT RAISE(ABORT,'injected crash'); END;").unwrap();
    assert!(migration::activate(&w, &snapshot).is_err());
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM migration_sources WHERE source_path='@activation'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
    assert_eq!(migration::inspect(&w).unwrap().backend, "shadow");
    c.execute_batch("DROP TRIGGER fail_activation").unwrap();
    migration::activate(&w, &snapshot).unwrap();
}

#[test]
fn active_repeat_never_resets_new_writes_and_backup_is_consistent() {
    let repo = committed_repo();
    let w = workspace(repo.path());
    append(&w, "s", "a");
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    migration::ensure(&w, &snapshot).unwrap();
    let accepted = JournalStore::open(&w, "s")
        .unwrap()
        .append(event("s", "sql-new", 2))
        .unwrap();
    migration::ensure(&w, &snapshot).unwrap();
    migration::import_shadow(&w, &snapshot).unwrap();
    migration::activate(&w, &snapshot).unwrap();
    assert_eq!(
        JournalStore::open(&w, "s")
            .unwrap()
            .replay()
            .unwrap()
            .last(),
        Some(&accepted)
    );
    assert_eq!(migration::inspect(&w).unwrap().generation, 1);
    let destination = backup.path().join("active.db");
    migration::backup(&w, &destination).unwrap();
    let restored = rusqlite::Connection::open(destination).unwrap();
    assert_eq!(
        restored
            .query_row("SELECT count(*) FROM journal_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert!(migration::backup(&w, &w.git_common_dir.join("bad.db")).is_err());
}

#[test]
fn late_legacy_writer_is_diagnosed_and_sql_preserved() {
    let repo = committed_repo();
    let w = workspace(repo.path());
    append(&w, "s", "a");
    let backup = tempfile::tempdir().unwrap();
    migration::ensure(&w, &backup.path().join("frozen")).unwrap();
    let journal = JournalStore::open(&w, "s").unwrap();
    journal.append(event("s", "sql-new", 2)).unwrap();
    // Simulate a pre-SQL executable still writing its old path; selector ignorance is deliberate.
    let path = w.git_dir.join("devmap/sessions/s/events.ndjson");
    let original = std::fs::read(&path).unwrap();
    let mut changed = original.clone();
    changed.extend_from_slice(b"late legacy writer\n");
    std::fs::write(&path, &changed).unwrap();
    assert!(
        migration::verify(&w)
            .unwrap_err()
            .to_string()
            .contains("forward recovery")
    );
    assert!(journal.append(event("s", "blocked", 3)).is_err());
    assert_eq!(migration::inspect(&w).unwrap().backend, "active");
    assert_eq!(
        database(&w)
            .query_row("SELECT count(*) FROM journal_records", [], |r| r
                .get::<_, i64>(0))
            .unwrap(),
        2
    );
    assert_eq!(std::fs::read(path).unwrap(), changed);
}

#[test]
fn frozen_tampering_and_wrong_repository_are_rejected() {
    let repo = committed_repo();
    let w = workspace(repo.path());
    append(&w, "s", "a");
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    let manifest = migration::freeze(&w, &snapshot, now()).unwrap();
    let other = committed_repo();
    assert!(migration::import_shadow(&workspace(other.path()), &snapshot).is_err());
    let f = manifest
        .files
        .iter()
        .find(|f| f.relative.ends_with("events.ndjson"))
        .unwrap();
    std::fs::write(
        snapshot.join(f.origin.to_string()).join(&f.relative),
        b"changed",
    )
    .unwrap();
    assert!(migration::import_shadow(&w, &snapshot).is_err());
}

#[test]
fn unlisted_frozen_artifacts_and_false_counts_cannot_activate() {
    let repo = committed_repo();
    let w = workspace(repo.path());
    append(&w, "s", "a");
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    let manifest = migration::freeze(&w, &snapshot, now()).unwrap();
    let pending = snapshot.join("unlisted.pending");
    std::fs::write(&pending, b"pending").unwrap();
    assert!(migration::import_shadow(&w, &snapshot).is_err());
    std::fs::remove_file(pending).unwrap();
    let mut changed = serde_json::to_value(&manifest).unwrap();
    changed["files"][0]["record_count"] = serde_json::json!(99);
    std::fs::write(
        snapshot.join("manifest.json"),
        serde_json::to_vec(&changed).unwrap(),
    )
    .unwrap();
    assert!(migration::import_shadow(&w, &snapshot).is_err());
}

#[test]
fn altered_projection_registry_and_provenance_prevent_activation() {
    for tamper in [
        "UPDATE presence_projection SET covered_sequence=0,covered_sha256=NULL",
        "UPDATE worktree_registry SET workspace_path='wrong'",
        "DELETE FROM migration_sources WHERE source_path NOT LIKE '@%'",
    ] {
        let repo = committed_repo();
        let w = workspace(repo.path());
        let r = append(&w, "s", "a");
        PresenceStore::open(&w)
            .unwrap()
            .observe(PresenceSignal::AcceptedRecords(&[r]), now())
            .unwrap();
        let backup = tempfile::tempdir().unwrap();
        let snapshot = backup.path().join("frozen");
        migration::freeze(&w, &snapshot, now()).unwrap();
        migration::import_shadow(&w, &snapshot).unwrap();
        database(&w).execute_batch(tamper).unwrap();
        assert!(
            migration::activate(&w, &snapshot).is_err(),
            "accepted {tamper}"
        );
        assert_eq!(migration::inspect(&w).unwrap().backend, "shadow");
    }
}

#[test]
fn unknown_schema_and_corrupt_databases_are_preserved() {
    for bytes in [Vec::new(), b"not sqlite".to_vec()] {
        let repo = committed_repo();
        let w = workspace(repo.path());
        let root = w.git_common_dir.join("devmap");
        std::fs::create_dir(&root).unwrap();
        let path = root.join("devmap.db");
        std::fs::write(&path, &bytes).unwrap();
        let backup = tempfile::tempdir().unwrap();
        assert!(migration::ensure(&w, &backup.path().join("frozen")).is_err());
        assert_eq!(std::fs::read(path).unwrap(), bytes);
    }
    let repo = committed_repo();
    let w = workspace(repo.path());
    let store = RepositoryStore::open(&w).unwrap();
    database(&w)
        .execute("UPDATE store_meta SET schema_version=999", [])
        .unwrap();
    let backup = tempfile::tempdir().unwrap();
    assert!(migration::ensure(&w, &backup.path().join("frozen")).is_err());
    assert!(store.path().exists());
}

#[test]
fn pending_watermark_and_presence_temporary_are_rejected() {
    for name in [
        "task-binding-watermarks.pending",
        "presence/v1/s.json.pending",
    ] {
        let repo = committed_repo();
        let w = workspace(repo.path());
        let path = w.git_common_dir.join("devmap").join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, b"pending").unwrap();
        let backup = tempfile::tempdir().unwrap();
        assert!(migration::ensure(&w, &backup.path().join("frozen")).is_err());
        assert_eq!(std::fs::read(path).unwrap(), b"pending");
    }
}

#[test]
fn active_selector_downgrade_and_missing_database_never_resume_legacy() {
    let repo = committed_repo();
    let w = workspace(repo.path());
    append(&w, "s", "a");
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    migration::ensure(&w, &snapshot).unwrap();
    database(&w)
        .execute("UPDATE store_meta SET backend_state='shadow'", [])
        .unwrap();
    assert!(JournalStore::open(&w, "s").is_err());
    assert!(migration::ensure(&w, &snapshot).is_err());
    database(&w)
        .execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    let path = w.git_common_dir.join("devmap/devmap.db");
    std::fs::rename(&path, backup.path().join("retained.db")).unwrap();
    assert!(JournalStore::open(&w, "s").is_err());
    assert!(migration::ensure(&w, &snapshot).is_err());
    assert!(!path.exists());
}

#[test]
fn comparison_preserves_complete_and_partial_inventory_timestamps() {
    let repo = committed_repo();
    let w = workspace(repo.path());
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    migration::freeze(&w, &snapshot, now()).unwrap();
    migration::import_shadow(&w, &snapshot).unwrap();
    for complete in [false, true] {
        let observed = "2026-09-07T10:00:00Z".to_string();
        let pair = migration::compare_snapshot_with_inventory(
            &w,
            &snapshot,
            &[],
            Some(observed.clone()),
            complete,
        )
        .unwrap();
        assert_eq!(pair.legacy.task_inventory_synced_at, Some(observed));
        assert_eq!(pair.legacy.task_observation.complete, complete);
        assert_eq!(
            serde_json::to_value(&pair.legacy).unwrap(),
            serde_json::to_value(&pair.sql).unwrap()
        );
    }
}

#[test]
fn reassigning_journal_to_another_valid_imported_origin_prevents_activation() {
    let repo = committed_repo();
    let linked = support::linked_worktree(repo.path(), "other-origin");
    let w = workspace(repo.path());
    let lw = workspace(linked.path());
    append(&w, "main-session", "main-event");
    append(&lw, "linked-session", "linked-event");
    let backup = tempfile::tempdir().unwrap();
    let snapshot = backup.path().join("frozen");
    migration::freeze(&w, &snapshot, now()).unwrap();
    migration::import_shadow(&w, &snapshot).unwrap();
    let c = database(&w);
    c.execute("UPDATE journal_sessions SET (worktree_id,incarnation,origin_path)=(SELECT worktree_id,incarnation,origin_path FROM journal_sessions WHERE session_id='linked-session') WHERE session_id='main-session'",[]).unwrap();
    assert!(
        c.prepare("PRAGMA foreign_key_check")
            .unwrap()
            .query([])
            .unwrap()
            .next()
            .unwrap()
            .is_none()
    );
    assert!(
        migration::activate(&w, &snapshot).is_err(),
        "internally consistent reassignment lost the original session provenance"
    );
    assert_eq!(migration::inspect(&w).unwrap().backend, "shadow");
    assert_eq!(
        c.query_row(
            "SELECT count(*) FROM migration_sources WHERE source_path='@activation'",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0
    );
}
