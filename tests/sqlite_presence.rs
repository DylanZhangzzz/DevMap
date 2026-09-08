mod support;
use devmap::{
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::{SourceGitInspector, SourceWorkspace},
    journal::JournalStore,
    presence::{PresenceSignal, PresenceStatus, PresenceStore},
    store::RepositoryStore,
};
use serde_json::json;
use support::committed_repo;
use time::{Duration, OffsetDateTime};
fn setup() -> (tempfile::TempDir, SourceWorkspace) {
    let d = committed_repo();
    let w = SourceGitInspector::open(d.path())
        .unwrap()
        .workspace()
        .unwrap();
    let s = RepositoryStore::open(&w).unwrap();
    rusqlite::Connection::open(s.path())
        .unwrap()
        .execute("UPDATE store_meta SET backend_state='active'", [])
        .unwrap();
    (d, w)
}
fn event(n: u64) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        "gap",
        EventType::CaptureGap,
        n,
        "2026-09-08T10:00:00Z",
        HostIdentity::new("test", "1").unwrap(),
        ActorIdentity::new("a", None).unwrap(),
        SessionContext::new("s", None, "fixture", None, None, None).unwrap(),
        json!({"reason":"test"}),
    )
    .unwrap()
}
#[test]
fn duplicate_projection_preserves_gap_lease_and_explicit_wait() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    let records = vec![j.append(event(1)).unwrap()];
    let p = PresenceStore::open(&w).unwrap();
    let now = OffsetDateTime::from_unix_timestamp(1788861600).unwrap();
    let first = p
        .observe(PresenceSignal::AcceptedRecords(&records), now)
        .unwrap();
    assert_eq!(
        p.observe(
            PresenceSignal::AcceptedRecords(&records),
            now + Duration::seconds(10)
        )
        .unwrap(),
        first
    );
    let waiting = p
        .observe(
            PresenceSignal::ExplicitWaiting {
                session_id: "s",
                activity_id: Some("wait"),
            },
            now + Duration::seconds(20),
        )
        .unwrap();
    assert_eq!(
        p.observe(
            PresenceSignal::AcceptedRecords(&records),
            now + Duration::seconds(30)
        )
        .unwrap(),
        waiting
    );
    let loaded = PresenceStore::open_existing(&w)
        .unwrap()
        .unwrap()
        .load_all();
    assert_eq!(loaded.records, vec![waiting.clone()]);
    assert_eq!(
        waiting.effective_at(now + Duration::seconds(200)).status,
        PresenceStatus::Stale
    );
    assert!(!w.git_common_dir.join("devmap/presence").exists());
}
#[test]
fn corrupted_sql_presence_has_warning() {
    let (_d, w) = setup();
    let records = vec![
        JournalStore::open(&w, "s")
            .unwrap()
            .append(event(1))
            .unwrap(),
    ];
    let p = PresenceStore::open(&w).unwrap();
    p.observe(
        PresenceSignal::AcceptedRecords(&records),
        OffsetDateTime::now_utc(),
    )
    .unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    let c = rusqlite::Connection::open(s.path()).unwrap();
    assert_eq!(
        c.execute("UPDATE presence_records SET record_json='{}'", [])
            .unwrap(),
        1
    );
    let report = p.load_all();
    assert!(report.records.is_empty());
    assert_eq!(report.warnings.len(), 1);
}
#[test]
fn capture_projection_failure_rolls_back_journal_and_generation() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    let c = rusqlite::Connection::open(s.path()).unwrap();
    c.execute_batch("CREATE TRIGGER reject_presence BEFORE INSERT ON presence_records BEGIN SELECT RAISE(ABORT,'injected projection failure'); END;").unwrap();
    assert!(
        j.append_capture_batch_with(OffsetDateTime::now_utc(), |n| Ok(vec![event(n)]))
            .is_err()
    );
    assert!(j.replay().unwrap().is_empty());
    assert_eq!(
        RepositoryStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        0
    );
    assert!(
        PresenceStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .load_all()
            .records
            .is_empty()
    );
    c.execute_batch("DROP TRIGGER reject_presence").unwrap();
    j.append_capture_batch_with(OffsetDateTime::now_utc(), |n| Ok(vec![event(n)]))
        .unwrap();
    assert_eq!(
        RepositoryStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        1
    );
}
#[test]
fn imported_explicit_wait_baseline_and_missing_journal_remain_readable() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    let records = j
        .append_capture_batch_with(OffsetDateTime::now_utc(), |n| Ok(vec![event(n)]))
        .unwrap();
    let p = PresenceStore::open(&w).unwrap();
    let waiting = p
        .observe(
            PresenceSignal::ExplicitWaiting {
                session_id: "s",
                activity_id: Some("explicit"),
            },
            OffsetDateTime::now_utc(),
        )
        .unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    let c = rusqlite::Connection::open(s.path()).unwrap();
    c.execute(
        "UPDATE presence_projection SET baseline_source='legacy_import'",
        [],
    )
    .unwrap();
    assert_eq!(
        p.observe(
            PresenceSignal::AcceptedRecords(&records),
            OffsetDateTime::now_utc() + Duration::seconds(30)
        )
        .unwrap(),
        waiting
    );
    c.execute_batch("DELETE FROM presence_projection;DELETE FROM journal_records;DELETE FROM journal_heads;DELETE FROM journal_sessions;").unwrap();
    assert_eq!(
        PresenceStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .load_all()
            .records,
        vec![waiting]
    );
    assert_eq!(
        devmap::journal::summarize_existing_sessions(&w, &["s".into()].into())["s"].integrity,
        devmap::journal::JournalIntegrity::Missing
    );
}
#[test]
fn atomic_capture_retry_does_not_advance_generation() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    let now = OffsetDateTime::now_utc();
    let first = j
        .append_capture_batch_with(now, |n| Ok(vec![event(n)]))
        .unwrap();
    let p = PresenceStore::open(&w).unwrap();
    let before = p.load_all();
    let second = j
        .append_capture_batch_with(now + Duration::seconds(60), |n| Ok(vec![event(n)]))
        .unwrap();
    assert_eq!(first, second);
    assert_eq!(p.load_all(), before);
    assert_eq!(
        RepositoryStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        1
    );
}
#[test]
fn hook_sql_projection_failure_is_an_acceptance_error() {
    use devmap::cli::{AdapterHost, HookHandleArgs};
    let (_d, w) = setup();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    rusqlite::Connection::open(s.path()).unwrap().execute_batch("CREATE TRIGGER reject_presence BEFORE INSERT ON presence_records BEGIN SELECT RAISE(ABORT,'injected failure'); END;").unwrap();
    let mut input = std::io::Cursor::new(
        serde_json::to_vec(&json!({"session_id":"s","event_id":"start","source":"startup"}))
            .unwrap(),
    );
    assert!(
        devmap::hook::handle_hook(
            HookHandleArgs {
                source: w.root.clone(),
                host: AdapterHost::Codex,
                event: "SessionStart".into()
            },
            &mut input
        )
        .is_err()
    );
    assert!(
        JournalStore::open(&w, "s")
            .unwrap()
            .replay()
            .unwrap()
            .is_empty()
    );
    assert_eq!(
        RepositoryStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        0
    );
}
#[test]
fn oversized_sql_presence_is_warned_not_missing() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    j.append_capture_batch_with(OffsetDateTime::now_utc(), |n| Ok(vec![event(n)]))
        .unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    rusqlite::Connection::open(s.path())
        .unwrap()
        .execute(
            "UPDATE presence_records SET record_json=?1",
            [
                serde_json::to_string(&"x".repeat(devmap::presence::MAX_PRESENCE_BYTES + 1))
                    .unwrap(),
            ],
        )
        .unwrap();
    let report = PresenceStore::open_existing(&w)
        .unwrap()
        .unwrap()
        .load_all();
    assert!(report.records.is_empty());
    assert_eq!(report.warnings.len(), 1);
    assert_eq!(report.warnings[0].subject_id.as_deref(), Some("s"));
}
