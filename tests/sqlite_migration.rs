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

fn schema2_legacy_identity_fixture() -> (
    tempfile::TempDir,
    tempfile::TempDir,
    SourceWorkspace,
    std::path::PathBuf,
    String,
    String,
) {
    let repo = committed_repo();
    let external = tempfile::tempdir().unwrap();
    let linked_path = external.path().join("old-origin");
    support::git(
        repo.path(),
        [
            "worktree",
            "add",
            "-b",
            "old-origin",
            linked_path.to_str().unwrap(),
        ],
    );
    let w = workspace(repo.path());
    let linked = workspace(&linked_path);
    let rows = devmap::worktrees::WorktreeScanner::scan(&w).unwrap();
    let main_id = rows
        .iter()
        .find(|row| row.root == w.root)
        .unwrap()
        .worktree_id
        .clone();
    let old_id = rows
        .iter()
        .find(|row| row.root == linked.root)
        .unwrap()
        .worktree_id
        .clone();
    let plans = devmap::route_plan::RoutePlanStore::open(&w).unwrap();
    for (request, id) in [("known-route", &main_id), ("unknown-route", &old_id)] {
        plans
            .set(devmap::route_plan::PlanInput {
                delivery: Default::default(),
                request_id: request.into(),
                route_id: None,
                expected_revision: 0,
                worktree_id: id.clone(),
                goal: request.into(),
                target_ref: None,
                milestones: vec![],
                source: "user".into(),
                abandoned: false,
            })
            .unwrap();
    }
    let mut dock = devmap::dock::DockService::open(&w.root).unwrap();
    for (path, at, timestamp) in [
        (&linked.root, "2026-09-08T10:00:00Z", now()),
        (
            &w.root,
            "2026-09-08T10:00:10Z",
            now() + time::Duration::seconds(10),
        ),
    ] {
        dock.replace_observed_tasks(
            vec![devmap::dock::ObservedTask {
                working_directory: None,
                subagents: None,
                lifecycle: devmap::dock::TaskLifecycle::Present,
                session_id: "01a00000-0000-7000-8000-000000000013".into(),
                display_title: "baseline task".into(),
                host: "local".into(),
                host_status: "active".into(),
                workspace_path: path.to_string_lossy().into_owned(),
                status: devmap::presence::PresenceStatus::Working,
                updated_at: at.into(),
            }],
            timestamp,
        )
        .unwrap();
    }
    drop(dock);
    // A watermark-only record carries no association. Keep its valid alternate
    // RFC3339 spelling to detect accidental cursor normalization/backfill.
    let watermarks = w.git_common_dir.join("devmap/task-binding-watermarks.json");
    let mut saved: serde_json::Value =
        serde_json::from_slice(&std::fs::read(&watermarks).unwrap()).unwrap();
    saved["observations"]
        .as_array_mut()
        .unwrap()
        .push(serde_json::json!([
            "local",
            "watermark-only",
            "2026-09-08T11:00:00+00:00"
        ]));
    std::fs::write(&watermarks, serde_json::to_vec(&saved).unwrap()).unwrap();
    support::git(
        &w.root,
        ["worktree", "remove", linked_path.to_str().unwrap()],
    );
    let snapshot = external.path().join("frozen");
    let manifest = migration::freeze(&w, &snapshot, now()).unwrap();
    assert_eq!(manifest.origins.len(), 1);
    assert_eq!(manifest.origins[0].worktree_id, main_id);
    (repo, external, w, snapshot, main_id, old_id)
}

#[test]
fn schema2_frozen_shadow_rejects_extra_registry_origin() {
    let (_repo, _external, w, snapshot, _, _) = schema2_legacy_identity_fixture();
    migration::import_shadow(&w, &snapshot).unwrap();
    let c = database(&w);
    c.execute("INSERT INTO worktree_registry(worktree_id,incarnation,git_dir,workspace_path) VALUES('extra-origin','extra-incarnation','extra-admin','extra-root')",[]).unwrap();
    assert!(migration::activate(&w, &snapshot).is_err());
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

#[test]
fn schema2_frozen_baseline_unknown_source_and_watermark_only_preserve_evidence() {
    let (_repo, _external, w, snapshot, main_id, old_id) = schema2_legacy_identity_fixture();
    let source_names = [
        "route-plans.jsonl",
        "task-bindings.jsonl",
        "task-binding-watermarks.json",
    ];
    let source_bytes =
        source_names.map(|name| std::fs::read(w.git_common_dir.join("devmap").join(name)).unwrap());
    migration::import_shadow(&w, &snapshot).unwrap();
    let c = database(&w);
    let incarnation: String = c
        .query_row(
            "SELECT incarnation FROM worktree_registry WHERE worktree_id=?1",
            [&main_id],
            |r| r.get(0),
        )
        .unwrap();
    let route_rows = c.prepare("SELECT json_array(r.request_id,l.qualification,l.worktree_id,l.incarnation) FROM route_records r JOIN route_origin_links l USING(route_id,revision) ORDER BY r.request_id")
        .unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().map(Result::unwrap).map(|row|serde_json::from_str::<serde_json::Value>(&row).unwrap()).collect::<Vec<_>>();
    assert_eq!(
        route_rows,
        vec![
            serde_json::json!(["known-route", "frozen_baseline", main_id, incarnation]),
            serde_json::json!(["unknown-route", "unknown", old_id, null])
        ]
    );
    let binding_rows = c.prepare("SELECT json_array(l.destination_worktree_id,l.destination_incarnation,l.destination_qualification,l.source_worktree_id,l.source_incarnation,l.source_qualification) FROM binding_records b JOIN binding_origin_links l USING(observation_id) ORDER BY b.observed_at")
        .unwrap().query_map([],|r|r.get::<_,String>(0)).unwrap().map(Result::unwrap).map(|row|serde_json::from_str::<serde_json::Value>(&row).unwrap()).collect::<Vec<_>>();
    assert_eq!(
        binding_rows,
        vec![
            serde_json::json!([old_id, null, "unknown", null, null, "not_applicable"]),
            serde_json::json!([
                main_id,
                incarnation,
                "frozen_baseline",
                old_id,
                null,
                "unknown"
            ])
        ]
    );
    let orphan_scope = serde_json::to_string(&("local", "watermark-only")).unwrap();
    let orphan:String = c.query_row("SELECT json_array(observed_at,current_worktree_id,current_incarnation,qualification,history_observation_id) FROM binding_origin_cursors WHERE source_scope=?1",[&orphan_scope],|r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&orphan).unwrap(),
        serde_json::json!(["2026-09-08T11:00:00+00:00", null, null, "unobserved", null])
    );
    let associated_scope =
        serde_json::to_string(&("local", "01a00000-0000-7000-8000-000000000013")).unwrap();
    let associated:String = c.query_row("SELECT json_array(observed_at,current_worktree_id,current_incarnation,qualification,history_observation_id IS NOT NULL) FROM binding_origin_cursors WHERE source_scope=?1",[&associated_scope],|r|r.get(0)).unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(&associated).unwrap(),
        serde_json::json!([
            "2026-09-08T10:00:10Z",
            main_id,
            incarnation,
            "frozen_baseline",
            1
        ])
    );
    let pair = migration::compare_snapshot(&w, &snapshot, &[]).unwrap();
    assert_eq!(
        serde_json::to_value(pair.legacy).unwrap(),
        serde_json::to_value(pair.sql).unwrap()
    );
    migration::activate(&w, &snapshot).unwrap();
    for (index, name) in source_names.iter().enumerate() {
        assert_eq!(
            std::fs::read(w.git_common_dir.join("devmap").join(name)).unwrap(),
            source_bytes[index]
        );
    }
    // A later native route must keep its independently accepted qualification
    // through verify and idempotent active import, not be recomputed as baseline.
    let plan = devmap::route_plan::RoutePlanStore::open(&w)
        .unwrap()
        .set(devmap::route_plan::PlanInput {
            delivery: Default::default(),
            request_id: "later-native".into(),
            route_id: None,
            expected_revision: 0,
            worktree_id: main_id,
            goal: "later native".into(),
            target_ref: None,
            milestones: vec![],
            source: "user".into(),
            abandoned: false,
        })
        .unwrap();
    let link_before:String = c.query_row("SELECT json_array(worktree_id,incarnation,qualification) FROM route_origin_links WHERE route_id=?1 AND revision=1",[&plan.route_id],|r|r.get(0)).unwrap();
    assert!(link_before.contains("native_verified"));
    migration::verify(&w).unwrap();
    migration::import_shadow(&w, &snapshot).unwrap();
    let link_after:String = c.query_row("SELECT json_array(worktree_id,incarnation,qualification) FROM route_origin_links WHERE route_id=?1 AND revision=1",[&plan.route_id],|r|r.get(0)).unwrap();
    assert_eq!(link_after, link_before);
}

#[test]
fn schema2_shadow_rejects_relabeling_frozen_route_as_native() {
    let (_repo, _external, w, snapshot, _, _) = schema2_legacy_identity_fixture();
    migration::import_shadow(&w, &snapshot).unwrap();
    let c = database(&w);
    let changed=c.execute("UPDATE route_origin_links SET qualification='native_verified' WHERE qualification='frozen_baseline'",[]).unwrap();
    assert_eq!(changed, 1);
    assert!(migration::activate(&w, &snapshot).is_err());
    assert_eq!(migration::inspect(&w).unwrap().backend, "shadow");
}
