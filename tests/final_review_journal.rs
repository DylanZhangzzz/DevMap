mod support;

use std::fs;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use devmap::canonical::canonical_json;
use devmap::events::{
    ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
};
use devmap::git::SourceGitInspector;
use devmap::journal::{JournalStore, MAX_JOURNAL_BYTES};
use fs2::FileExt;
use serde_json::json;
use support::committed_repo;

fn event(session: &str, sequence: u64, id: &str) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        id,
        EventType::EvidenceRecorded,
        sequence,
        "2026-08-27T16:00:00Z",
        HostIdentity::new("test-host", "1.0.0").unwrap(),
        ActorIdentity::new("agent-1", None).unwrap(),
        SessionContext::new(session, None, "fixture-repository", None, None, None).unwrap(),
        json!({"kind": "test"}),
    )
    .unwrap()
}

#[test]
fn append_rejects_an_event_for_a_different_session_directory() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-a").unwrap();

    let error = store.append(event("session-b", 1, "evt-1")).unwrap_err();

    assert!(matches!(
        error,
        devmap::error::DevMapError::SessionMismatch { .. }
    ));
    assert!(store.replay().unwrap().is_empty());
}

#[test]
fn replay_recovers_a_durable_intent_immediately_after_a_crash() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "crash-session").unwrap();
    let session = workspace.git_dir.join("devmap/sessions/crash-session");
    let intent = canonical_json(&json!({
        "events": [event("crash-session", 1, "evt-after-crash")]
    }))
    .unwrap();
    fs::write(session.join("events.intent"), intent).unwrap();

    let replayed = store.replay().unwrap();

    assert_eq!(replayed.len(), 1);
    assert_eq!(replayed[0].event.event_id(), "evt-after-crash");
    assert!(!session.join("events.intent").exists());
}

#[test]
fn public_replay_waits_for_the_writer_lock_before_reading() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "locked-session").unwrap();
    store
        .append(event("locked-session", 1, "evt-before-lock"))
        .unwrap();
    let lock_path = workspace
        .git_dir
        .join("devmap/sessions/locked-session/events.lock");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .unwrap();
    lock.lock_exclusive().unwrap();

    let (sender, receiver) = mpsc::channel();
    let reader = store.clone();
    let worker = thread::spawn(move || sender.send(reader.replay()).unwrap());
    assert!(
        receiver.recv_timeout(Duration::from_millis(150)).is_err(),
        "reader bypassed the active journal writer lock"
    );
    lock.unlock().unwrap();

    let replayed = receiver
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    worker.join().unwrap();
    assert_eq!(replayed.len(), 1);
}

#[test]
fn journal_size_is_bounded_before_replay_allocates_it() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "large-session").unwrap();
    let path = workspace
        .git_dir
        .join("devmap/sessions/large-session/events.ndjson");
    let file = fs::File::create(path).unwrap();
    file.set_len(MAX_JOURNAL_BYTES as u64 + 1).unwrap();

    let error = store.replay().unwrap_err();
    assert!(matches!(
        error,
        devmap::error::DevMapError::ResourceLimit { .. }
    ));
}

#[test]
fn replacing_an_open_session_directory_is_refused_without_writing_the_replacement() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "swapped-session").unwrap();
    let original = workspace.git_dir.join("devmap/sessions/swapped-session");
    let displaced = workspace
        .git_dir
        .join("devmap/sessions/swapped-session-original");
    fs::rename(&original, &displaced).unwrap();
    fs::create_dir(&original).unwrap();

    let error = store
        .append(event("swapped-session", 1, "evt-after-swap"))
        .expect_err("session identity changed after the store was opened");

    assert!(error.to_string().contains("unsafe") || error.to_string().contains("identity"));
    assert!(fs::read_dir(&original).unwrap().next().is_none());
    assert!(fs::read_dir(&displaced).unwrap().next().is_none());
}

#[cfg(unix)]
#[test]
fn journal_refuses_a_symlinked_session_component() {
    use std::os::unix::fs::symlink;

    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let outside = tempfile::tempdir().unwrap();
    let sessions = workspace.git_dir.join("devmap/sessions");
    fs::create_dir_all(&sessions).unwrap();
    symlink(outside.path(), sessions.join("redirected")).unwrap();

    assert!(JournalStore::open(&workspace, "redirected").is_err());
    assert!(fs::read_dir(outside.path()).unwrap().next().is_none());
}
