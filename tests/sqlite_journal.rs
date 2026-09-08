mod support;
use devmap::{
    events::{
        ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
    },
    git::{SourceGitInspector, SourceWorkspace},
    journal::{JournalIntegrity, JournalStore, summarize_existing_sessions},
    store::RepositoryStore,
};
use serde_json::json;
use support::committed_repo;
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
fn event(seq: u64, id: &str, time: &str) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        id,
        EventType::CaptureGap,
        seq,
        time,
        HostIdentity::new("test", "1").unwrap(),
        ActorIdentity::new("a", None).unwrap(),
        SessionContext::new("s", None, "fixture", None, None, None).unwrap(),
        json!({"reason":"gap"}),
    )
    .unwrap()
}
#[test]
fn sql_journal_restart_retry_and_atomic_partial() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    let a = j
        .append_batch_with(|n| Ok(vec![event(n, "a", "2026-09-08T10:00:00Z")]))
        .unwrap();
    assert!(!w.git_dir.join("devmap/sessions/s").exists());
    assert_eq!(
        RepositoryStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        1
    );
    assert_eq!(
        j.append_batch_with(|n| Ok(vec![event(n, "a", "2026-09-08T11:00:00Z")]))
            .unwrap(),
        a
    );
    assert!(
        j.append_batch_with(|n| Ok(vec![
            event(n, "a", "2026-09-08T10:00:00Z"),
            event(n + 1, "b", "2026-09-08T10:00:00Z")
        ]))
        .is_err()
    );
    assert_eq!(JournalStore::open(&w, "s").unwrap().replay().unwrap(), a);
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
fn sql_tampered_row_is_corrupt() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    j.append(event(1, "a", "2026-09-08T10:00:00Z")).unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    let c = rusqlite::Connection::open(s.path()).unwrap();
    assert_eq!(
        c.execute("UPDATE journal_records SET byte_length=0", [])
            .unwrap(),
        1
    );
    assert!(j.replay().is_err());
    assert_eq!(
        summarize_existing_sessions(&w, &["s".into()].into())["s"].integrity,
        JournalIntegrity::Corrupt
    );
}
#[test]
fn linked_session_origins_conflict_and_recreated_root_cannot_inherit() {
    let (d, w) = setup();
    let linked = support::linked_worktree(d.path(), "linked");
    let lw = SourceGitInspector::open(linked.path())
        .unwrap()
        .workspace()
        .unwrap();
    JournalStore::open(&w, "s")
        .unwrap()
        .append(event(1, "a", "2026-09-08T10:00:00Z"))
        .unwrap();
    assert!(
        JournalStore::open(&lw, "s")
            .unwrap()
            .append(event(1, "a", "2026-09-08T10:00:00Z"))
            .is_err()
    );
    let j = JournalStore::open(&lw, "local").unwrap();
    j.append_batch_with(|n| {
        let mut v = serde_json::to_value(event(n, "local", "2026-09-08T10:00:00Z")).unwrap();
        v["context"]["session_id"] = json!("local");
        Ok(vec![serde_json::from_value(v).unwrap()])
    })
    .unwrap();
    let parked = linked.path().with_extension("parked");
    std::fs::rename(linked.path(), &parked).unwrap();
    std::fs::create_dir(linked.path()).unwrap();
    std::fs::copy(parked.join(".git"), linked.path().join(".git")).unwrap();
    let replacement = SourceGitInspector::open(linked.path())
        .unwrap()
        .workspace()
        .unwrap();
    assert!(
        JournalStore::open(&replacement, "local")
            .unwrap()
            .replay()
            .is_err()
    );
    std::fs::remove_dir_all(&parked).unwrap();
}
#[test]
fn concurrent_append_and_retry_preserve_chain() {
    let (_d, w) = setup();
    let mut workers = vec![];
    for i in 0..4 {
        let w = w.clone();
        workers.push(std::thread::spawn(move || {
            let j = JournalStore::open(&w, "s").unwrap();
            j.append_batch_with(|n| Ok(vec![event(n, &format!("e{i}"), "2026-09-08T10:00:00Z")]))
                .unwrap()
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    let j = JournalStore::open(&w, "s").unwrap();
    let original = j.replay().unwrap();
    assert_eq!(original.len(), 4);
    let mut workers = vec![];
    for _ in 0..4 {
        let j = j.clone();
        workers.push(std::thread::spawn(move || {
            j.append_batch_with(|n| Ok(vec![event(n, "e0", "2026-09-08T11:00:00Z")]))
                .unwrap()
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }
    assert_eq!(j.replay().unwrap(), original);
    assert_eq!(
        RepositoryStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .generation()
            .unwrap(),
        4
    );
}
#[test]
fn changed_payload_rejected_and_direct_append_has_no_presence() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    j.append(event(1, "a", "2026-09-08T10:00:00Z")).unwrap();
    assert!(
        j.append_batch_with(|n| {
            let mut v = serde_json::to_value(event(n, "a", "2026-09-08T10:00:00Z")).unwrap();
            v["payload"]["reason"] = json!("changed");
            Ok(vec![serde_json::from_value(v).unwrap()])
        })
        .is_err()
    );
    assert!(
        devmap::presence::PresenceStore::open_existing(&w)
            .unwrap()
            .unwrap()
            .load_all()
            .records
            .is_empty()
    );
}
#[test]
fn frozen_sources_reject_incomplete_pending_and_duplicate_origins_without_writes() {
    use devmap::journal::{FrozenJournalSource, parse_frozen_journal_sources};
    let d = committed_repo();
    let w = SourceGitInspector::open(d.path())
        .unwrap()
        .workspace()
        .unwrap();
    JournalStore::open(&w, "s")
        .unwrap()
        .append(event(1, "a", "2026-09-08T10:00:00Z"))
        .unwrap();
    let path = w.git_dir.join("devmap/sessions/s/events.ndjson");
    let original = std::fs::read(&path).unwrap();
    let mut source = FrozenJournalSource {
        session_id: "s".into(),
        origin_path: w.git_dir.clone(),
        files: [("events.ndjson".into(), original.clone())].into(),
    };
    assert_eq!(
        parse_frozen_journal_sources(&[source.clone()]).unwrap()[0]
            .records
            .len(),
        1
    );
    source.files.get_mut("events.ndjson").unwrap().pop();
    assert!(parse_frozen_journal_sources(&[source.clone()]).is_err());
    source
        .files
        .insert("events.ndjson".into(), original.clone());
    source.files.insert("events.intent".into(), b"{}".to_vec());
    assert!(parse_frozen_journal_sources(&[source.clone()]).is_err());
    source.files.remove("events.intent");
    let mut other = source.clone();
    other.origin_path = w.git_dir.join("worktrees/other");
    assert!(parse_frozen_journal_sources(&[source, other]).is_err());
    assert_eq!(std::fs::read(path).unwrap(), original);
}
#[test]
fn deleting_accepted_tail_is_corrupt() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    j.append(event(1, "a", "2026-09-08T10:00:00Z")).unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    let c = rusqlite::Connection::open(s.path()).unwrap();
    c.execute("DELETE FROM journal_records", []).unwrap();
    assert!(j.replay().is_err());
    assert_eq!(
        summarize_existing_sessions(&w, &["s".into()].into())["s"].integrity,
        JournalIntegrity::Corrupt
    );
}
#[test]
fn altered_head_is_corrupt() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    j.append(event(1, "a", "2026-09-08T10:00:00Z")).unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    rusqlite::Connection::open(s.path())
        .unwrap()
        .execute("UPDATE journal_heads SET byte_length=byte_length+1", [])
        .unwrap();
    assert!(j.replay().is_err());
    assert_eq!(
        summarize_existing_sessions(&w, &["s".into()].into())["s"].integrity,
        JournalIntegrity::Corrupt
    );
}
#[test]
fn valid_empty_session_is_distinct_from_missing() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    j.append(event(1, "a", "2026-09-08T10:00:00Z")).unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    let c = rusqlite::Connection::open(s.path()).unwrap();
    c.execute("DELETE FROM journal_records", []).unwrap();
    c.execute(
        "UPDATE journal_heads SET record_count=0,last_sha256=NULL,byte_length=0",
        [],
    )
    .unwrap();
    let summaries = summarize_existing_sessions(&w, &["s".into(), "missing".into()].into());
    assert_eq!(summaries["s"].integrity, JournalIntegrity::Verified);
    assert_eq!(summaries["s"].records, 0);
    assert_eq!(summaries["missing"].integrity, JournalIntegrity::Missing);
}
#[test]
fn retired_incarnation_refuses_new_acceptance() {
    let (_d, w) = setup();
    let j = JournalStore::open(&w, "s").unwrap();
    j.append(event(1, "a", "2026-09-08T10:00:00Z")).unwrap();
    let s = RepositoryStore::open_existing(&w).unwrap().unwrap();
    rusqlite::Connection::open(s.path())
        .unwrap()
        .execute(
            "UPDATE worktree_registry SET retired_at='2026-09-08T11:00:00Z'",
            [],
        )
        .unwrap();
    assert!(j.append(event(2, "b", "2026-09-08T11:00:00Z")).is_err());
    assert_eq!(
        summarize_existing_sessions(&w, &["s".into()].into())["s"].records,
        1
    );
}

fn acceptance_state(c: &rusqlite::Connection) -> (i64, String, String, i64, i64, i64) {
    c.query_row(
        "SELECT generation,
         (SELECT coalesce(group_concat(record_json,''),'') FROM journal_records),
         (SELECT coalesce(group_concat(record_json,''),'') FROM presence_records),
         (SELECT count(*) FROM journal_sessions),
         (SELECT count(*) FROM journal_heads),
         (SELECT count(*) FROM presence_projection)
         FROM store_meta WHERE singleton=1",
        [],
        |r| {
            Ok((
                r.get(0)?,
                r.get(1)?,
                r.get(2)?,
                r.get(3)?,
                r.get(4)?,
                r.get(5)?,
            ))
        },
    )
    .unwrap()
}

fn new_session_event(sequence: u64) -> EventEnvelope {
    let mut value =
        serde_json::to_value(event(sequence, "new-event", "2026-09-08T11:00:00Z")).unwrap();
    value["context"]["session_id"] = json!("new-session");
    serde_json::from_value(value).unwrap()
}

#[test]
fn retired_registry_refuses_new_session_capture_before_build() {
    let (_d, w) = setup();
    JournalStore::open(&w, "s")
        .unwrap()
        .append_capture_batch_with(time::OffsetDateTime::now_utc(), |n| {
            Ok(vec![event(n, "first", "2026-09-08T10:00:00Z")])
        })
        .unwrap();
    let store = RepositoryStore::open_existing(&w).unwrap().unwrap();
    let c = rusqlite::Connection::open(store.path()).unwrap();
    c.execute(
        "UPDATE worktree_registry SET retired_at='2026-09-08T11:00:00Z'",
        [],
    )
    .unwrap();
    let before = acceptance_state(&c);
    let built = std::cell::Cell::new(false);
    let result = JournalStore::open(&w, "new-session")
        .unwrap()
        .append_capture_batch_with(time::OffsetDateTime::now_utc(), |n| {
            built.set(true);
            Ok(vec![new_session_event(n)])
        });
    assert!(
        result.is_err(),
        "new session must not bypass a retired incarnation"
    );
    assert!(
        !built.get(),
        "registry must be validated before building events"
    );
    assert_eq!(acceptance_state(&c), before);
}

#[test]
fn inconsistent_registry_refuses_new_session_before_build() {
    for mutation in [
        "UPDATE worktree_registry SET git_dir='wrong-origin'",
        "UPDATE worktree_registry SET workspace_path='wrong-workspace'",
    ] {
        let (_d, w) = setup();
        JournalStore::open(&w, "s")
            .unwrap()
            .append(event(1, "first", "2026-09-08T10:00:00Z"))
            .unwrap();
        let store = RepositoryStore::open_existing(&w).unwrap().unwrap();
        let c = rusqlite::Connection::open(store.path()).unwrap();
        c.execute(mutation, []).unwrap();
        let before = acceptance_state(&c);
        let built = std::cell::Cell::new(false);
        let result = JournalStore::open(&w, "new-session")
            .unwrap()
            .append_batch_with(|n| {
                built.set(true);
                Ok(vec![new_session_event(n)])
            });
        assert!(result.is_err(), "registry mismatch must reject: {mutation}");
        assert!(
            !built.get(),
            "registry must be validated before building events"
        );
        assert_eq!(acceptance_state(&c), before);
    }
}
