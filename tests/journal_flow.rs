mod support;

use std::fs;
use std::io::Write;
use std::path::Path;

use devmap::canonical::canonical_json;
use devmap::events::{
    ActorIdentity, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity, SessionContext,
};
use devmap::git::SourceGitInspector;
use devmap::journal::JournalStore;
use serde_json::json;
use support::{committed_repo, git, linked_worktree};

fn event(sequence: u64, event_id: &str) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        event_id,
        EventType::EvidenceRecorded,
        sequence,
        "2026-08-27T16:00:00Z",
        HostIdentity::new("test-host", "1.0.0").unwrap(),
        ActorIdentity::new("agent-1", None).unwrap(),
        SessionContext::new("session-1", None, "fixture-repository", None, None, None).unwrap(),
        json!({"kind": "test"}),
    )
    .unwrap()
}

fn source_snapshot(root: &Path) -> Vec<String> {
    vec![
        git(root, ["rev-parse", "HEAD"]),
        git(root, ["diff", "--cached", "--binary"]),
        git(root, ["symbolic-ref", "--short", "-q", "HEAD"]),
        git(root, ["for-each-ref", "--format=%(refname):%(objectname)"]),
        git(root, ["config", "--local", "--list"]),
    ]
}

fn journal_path(git_dir: &Path, session_id: &str) -> std::path::PathBuf {
    git_dir
        .join("devmap")
        .join("sessions")
        .join(session_id)
        .join("events.ndjson")
}

fn intent_path(git_dir: &Path, session_id: &str) -> std::path::PathBuf {
    git_dir
        .join("devmap")
        .join("sessions")
        .join(session_id)
        .join("events.intent")
}

fn lock_path(git_dir: &Path, session_id: &str) -> std::path::PathBuf {
    git_dir
        .join("devmap")
        .join("sessions")
        .join(session_id)
        .join("events.lock")
}

fn write_intent(git_dir: &Path, session_id: &str, events: &[EventEnvelope]) {
    let bytes = canonical_json(&json!({"events": events})).unwrap();
    fs::write(intent_path(git_dir, session_id), bytes).unwrap();
}

fn assert_recovered_batch(store: &JournalStore, expected_ids: &[&str]) {
    let records = store.replay().unwrap();
    assert_eq!(records.len(), expected_ids.len());
    assert_eq!(
        records
            .iter()
            .map(|record| record.event.event_id())
            .collect::<Vec<_>>(),
        expected_ids
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        (1..=expected_ids.len() as u64).collect::<Vec<_>>()
    );
}

#[test]
fn abandoned_sentinel_file_does_not_block_a_later_append() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    fs::write(
        lock_path(&workspace.git_dir, "session-1"),
        b"abandoned process",
    )
    .unwrap();

    store.append(event(1, "evt-1")).unwrap();

    assert_recovered_batch(&store, &["evt-1"]);
}

#[test]
fn durable_intent_without_records_is_completed_before_the_next_append() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    write_intent(
        &workspace.git_dir,
        "session-1",
        &[event(1, "evt-1"), event(2, "evt-2")],
    );

    store.append(event(3, "evt-3")).unwrap();

    assert_recovered_batch(&store, &["evt-1", "evt-2", "evt-3"]);
    assert!(!intent_path(&workspace.git_dir, "session-1").exists());
}

#[test]
fn durable_intent_with_a_valid_prefix_is_completed_without_duplicates() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    store.append(event(1, "evt-1")).unwrap();
    write_intent(
        &workspace.git_dir,
        "session-1",
        &[event(1, "evt-1"), event(2, "evt-2")],
    );

    store.append(event(3, "evt-3")).unwrap();

    assert_recovered_batch(&store, &["evt-1", "evt-2", "evt-3"]);
}

#[test]
fn durable_intent_truncates_only_its_torn_final_record_then_completes() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    store.append(event(1, "evt-1")).unwrap();
    write_intent(
        &workspace.git_dir,
        "session-1",
        &[event(1, "evt-1"), event(2, "evt-2")],
    );

    let reference_repo = committed_repo();
    let reference_workspace = SourceGitInspector::open(reference_repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let reference = JournalStore::open(&reference_workspace, "session-1").unwrap();
    reference.append(event(1, "evt-1")).unwrap();
    reference.append(event(2, "evt-2")).unwrap();
    let second_line = fs::read(journal_path(&reference_workspace.git_dir, "session-1"))
        .unwrap()
        .split(|byte| *byte == b'\n')
        .nth(1)
        .unwrap()
        .to_vec();
    let path = journal_path(&workspace.git_dir, "session-1");
    fs::OpenOptions::new()
        .append(true)
        .open(path)
        .unwrap()
        .write_all(&second_line[..second_line.len() / 2])
        .unwrap();

    store.append(event(3, "evt-3")).unwrap();

    assert_recovered_batch(&store, &["evt-1", "evt-2", "evt-3"]);
}

#[test]
fn durable_intent_for_an_already_complete_batch_is_only_cleaned_up() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    store.append(event(1, "evt-1")).unwrap();
    store.append(event(2, "evt-2")).unwrap();
    write_intent(
        &workspace.git_dir,
        "session-1",
        &[event(1, "evt-1"), event(2, "evt-2")],
    );

    store.append(event(3, "evt-3")).unwrap();

    assert_recovered_batch(&store, &["evt-1", "evt-2", "evt-3"]);
    assert!(!intent_path(&workspace.git_dir, "session-1").exists());
}

#[test]
fn append_batch_refuses_an_empty_durable_intent() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();

    assert!(store.append_batch_with(|_| Ok(Vec::new())).is_err());
    assert!(!intent_path(&workspace.git_dir, "session-1").exists());
}

#[test]
fn journal_uses_each_worktrees_resolved_git_directory() {
    let repository = committed_repo();
    let linked = linked_worktree(repository.path(), "feature/journal");
    let primary = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let secondary = SourceGitInspector::open(linked.path())
        .unwrap()
        .workspace()
        .unwrap();

    assert_ne!(primary.git_dir, secondary.git_dir);

    JournalStore::open(&primary, "session-1")
        .unwrap()
        .append(event(1, "evt-primary"))
        .unwrap();
    JournalStore::open(&secondary, "session-1")
        .unwrap()
        .append(event(1, "evt-secondary"))
        .unwrap();

    assert!(journal_path(&primary.git_dir, "session-1").is_file());
    assert!(journal_path(&secondary.git_dir, "session-1").is_file());
}

#[test]
fn journal_appends_a_contiguous_hash_chain_without_mutating_source_git() {
    let repository = committed_repo();
    let before = source_snapshot(repository.path());
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();

    let first = store.append(event(1, "evt-1")).unwrap();
    let second = store.append(event(2, "evt-2")).unwrap();
    let replayed = store.replay().unwrap();

    assert_eq!(replayed, vec![first.clone(), second.clone()]);
    assert_eq!(first.previous_sha256, None);
    assert_eq!(
        second.previous_sha256.as_deref(),
        Some(first.sha256.as_str())
    );
    assert_ne!(first.sha256, second.sha256);
    assert_eq!(before, source_snapshot(repository.path()));

    let bytes = fs::read(journal_path(&workspace.git_dir, "session-1")).unwrap();
    assert!(bytes.ends_with(b"\n"));
}

#[test]
fn replay_rejects_tampered_records_and_invalid_sequence_streams() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    store.append(event(1, "evt-1")).unwrap();
    store.append(event(2, "evt-2")).unwrap();
    let path = journal_path(&workspace.git_dir, "session-1");
    let original = fs::read_to_string(&path).unwrap();

    let modified = original.replacen("\"sha256\":\"", "\"sha256\":\"0", 1);
    fs::write(&path, modified).unwrap();
    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));

    fs::write(&path, &original).unwrap();
    let mut duplicate: Vec<serde_json::Value> = original
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    duplicate[1]["sequence"] = json!(1);
    fs::write(
        &path,
        duplicate
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n")
            + "\n",
    )
    .unwrap();
    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));

    fs::write(&path, &original).unwrap();
    let mut skipped: Vec<serde_json::Value> = original
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    skipped[1]["sequence"] = json!(3);
    fs::write(
        &path,
        skipped
            .iter()
            .map(serde_json::to_string)
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
            .join("\n")
            + "\n",
    )
    .unwrap();
    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));

    fs::write(&path, b"not-json\n").unwrap();
    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));
}

#[test]
fn append_rejects_non_next_sequences_and_accepts_an_equivalent_identifiable_retry() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    store.append(event(1, "evt-1")).unwrap();

    assert!(store.append(event(1, "evt-duplicate-sequence")).is_err());
    let retry = store.append(event(2, "evt-1")).unwrap();
    assert_eq!(retry.sequence, 1);
    assert_eq!(store.replay().unwrap().len(), 1);
    assert!(store.append(event(3, "evt-skipped")).is_err());
}

#[test]
fn journal_rejects_session_ids_that_are_not_one_normal_path_component() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();

    for session_id in [
        "",
        ".",
        "..",
        "one/two",
        "one\\two",
        "C:escape",
        "C:\\escape",
    ] {
        assert!(
            JournalStore::open(&workspace, session_id).is_err(),
            "{session_id:?} must not escape the session root"
        );
    }
}

#[test]
fn replay_requires_newline_delimited_canonical_records_without_unknown_fields() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    store.append(event(1, "evt-1")).unwrap();
    let path = journal_path(&workspace.git_dir, "session-1");
    let original = fs::read_to_string(&path).unwrap();

    fs::write(&path, original.trim_end_matches('\n')).unwrap();
    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));

    fs::write(&path, original.replacen('{', "{ ", 1)).unwrap();
    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));

    let value: serde_json::Value = serde_json::from_str(original.trim_end()).unwrap();
    let reordered = format!(
        "{{\"sha256\":{},\"sequence\":{},\"event\":{},\"previous_sha256\":{}}}\n",
        value["sha256"], value["sequence"], value["event"], value["previous_sha256"]
    );
    fs::write(&path, reordered).unwrap();
    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));

    let unknown = format!(
        "{},\"extra\":true}}\n",
        original.trim_end().trim_end_matches('}')
    );
    fs::write(&path, unknown).unwrap();
    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));
}

#[test]
fn replay_rejects_empty_complete_records() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let store = JournalStore::open(&workspace, "session-1").unwrap();
    store.append(event(1, "evt-1")).unwrap();
    let path = journal_path(&workspace.git_dir, "session-1");
    fs::write(path, b"\n").unwrap();

    assert!(store.replay().unwrap_err().to_string().contains("corrupt"));
}
