mod support;

use devmap::events::{
    ActorIdentity, CaptureGrade, EVENT_SCHEMA_VERSION, EventEnvelope, EventType, HostIdentity,
    SessionContext,
};
use devmap::git::SourceGitInspector;
use devmap::journal::JournalStore;
use devmap::presence::{
    Confidence, PresenceSignal, PresenceStatus, PresenceStore, StatusSource, project_status,
};
use serde_json::{Value, json};
use std::fs;
use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[test]
fn lease_expiry_is_stale_and_never_completed() {
    let mut record = support::presence_record(PresenceStatus::Working);
    record.lease_expires_at = Some("2026-09-02T12:00:00Z".into());
    let reduced =
        record.effective_at(OffsetDateTime::parse("2026-09-02T12:00:01Z", &Rfc3339).unwrap());
    assert_eq!(reduced.status, PresenceStatus::Stale);
    assert_eq!(reduced.status_source, StatusSource::Lease);
    assert_eq!(reduced.confidence, Confidence::Leased);
}

#[test]
fn capture_events_have_explicit_non_guessing_status_transitions() {
    assert_eq!(
        project_status(None, &EventType::SessionStarted),
        PresenceStatus::Starting
    );
    assert_eq!(
        project_status(Some(PresenceStatus::Starting), &EventType::ToolRequested),
        PresenceStatus::Working
    );
    assert_eq!(
        project_status(Some(PresenceStatus::Working), &EventType::TurnCompleted),
        PresenceStatus::Idle
    );
    assert_eq!(
        project_status(Some(PresenceStatus::Idle), &EventType::SessionStopped),
        PresenceStatus::Completed
    );
}

#[test]
fn accepted_records_and_explicit_waiting_round_trip_canonically() {
    let (_repo, store, _starting, _root) = observed_presence();
    let now = OffsetDateTime::parse("2026-09-02T12:00:00Z", &Rfc3339).unwrap();

    let waiting = store
        .observe(
            PresenceSignal::ExplicitWaiting {
                session_id: "session-presence",
                activity_id: Some("approval-42"),
            },
            now,
        )
        .unwrap();
    assert_eq!(waiting.status, PresenceStatus::Waiting);
    assert_eq!(waiting.status_source, StatusSource::HostExplicit);
    assert_eq!(waiting.current_activity_id.as_deref(), Some("approval-42"));

    let report = store.load_all();
    assert!(report.warnings.is_empty());
    assert!(!report.truncated);
    assert_eq!(report.records, vec![waiting]);
}

#[test]
fn traversal_and_mismatched_file_identity_are_rejected() {
    let (_repo, store, starting, root) = observed_presence();
    let error = store
        .observe(
            PresenceSignal::ExplicitWaiting {
                session_id: "../escape",
                activity_id: None,
            },
            OffsetDateTime::parse("2026-09-02T12:00:00Z", &Rfc3339).unwrap(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        devmap::error::DevMapError::InvalidPresence(_)
    ));

    fs::write(
        root.join("different-session.json"),
        devmap::canonical::canonical_json(&starting).unwrap(),
    )
    .unwrap();
    let report = store.load_all();
    assert_eq!(report.records.len(), 1);
    assert!(report.warnings.iter().any(|warning| {
        warning.code == "presence_record_invalid"
            && warning.subject_id.as_deref() == Some("different-session")
    }));
}

#[test]
fn loader_skips_wrong_repository_invalid_combinations_and_oversized_files() {
    let (_repo, store, mut record, root) = observed_presence();

    record.session_id = "wrong-repository".into();
    record.repository_id = format!("sha256-{}", "f".repeat(64));
    fs::write(
        root.join("wrong-repository.json"),
        devmap::canonical::canonical_json(&record).unwrap(),
    )
    .unwrap();

    record.session_id = "invalid-completed".into();
    record.repository_id = store.load_all().records[0].repository_id.clone();
    record.status = PresenceStatus::Completed;
    record.status_source = StatusSource::Lease;
    record.lease_expires_at = Some("2026-09-02T12:02:00Z".into());
    fs::write(
        root.join("invalid-completed.json"),
        devmap::canonical::canonical_json(&record).unwrap(),
    )
    .unwrap();
    fs::write(
        root.join("oversized.json"),
        vec![b'x'; devmap::presence::MAX_PRESENCE_BYTES + 1],
    )
    .unwrap();

    let report = store.load_all();
    assert_eq!(report.records.len(), 1);
    assert_eq!(report.warnings.len(), 3);
}

#[test]
fn loader_bounds_the_number_of_presence_records() {
    let (_repo, store, mut record, root) = observed_presence();
    fs::remove_file(root.join("session-presence.json")).unwrap();
    for index in 0..=devmap::presence::MAX_PRESENCE_RECORDS {
        record.session_id = format!("bounded-{index:04}");
        fs::write(
            root.join(format!("{}.json", record.session_id)),
            devmap::canonical::canonical_json(&record).unwrap(),
        )
        .unwrap();
    }

    let report = store.load_all();
    assert!(report.truncated);
    assert_eq!(report.records.len(), devmap::presence::MAX_PRESENCE_RECORDS);
}

#[test]
fn concurrent_updates_to_one_session_do_not_lose_gap_counts() {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let journal = JournalStore::open(&workspace, "concurrent-session").unwrap();
    let first = journal
        .append(presence_event(
            &workspace,
            "concurrent-session",
            "gap-1",
            1,
            EventType::CaptureGap,
        ))
        .unwrap();
    let second = journal
        .append(presence_event(
            &workspace,
            "concurrent-session",
            "gap-2",
            2,
            EventType::CaptureGap,
        ))
        .unwrap();
    let barrier = Arc::new(Barrier::new(3));
    let mut workers = Vec::new();
    for record in [first, second] {
        let store = PresenceStore::open(&workspace).unwrap();
        let barrier = Arc::clone(&barrier);
        workers.push(thread::spawn(move || {
            barrier.wait();
            store
                .observe(
                    PresenceSignal::AcceptedRecords(&[record]),
                    OffsetDateTime::parse("2026-09-02T12:00:00Z", &Rfc3339).unwrap(),
                )
                .unwrap();
        }));
    }
    barrier.wait();
    for worker in workers {
        worker.join().unwrap();
    }

    let report = PresenceStore::open(&workspace).unwrap().load_all();
    assert_eq!(report.records.len(), 1);
    assert_eq!(report.records[0].gap_count, 2);
}

#[test]
fn presence_schema_rejects_transcript_and_tool_content_fields() {
    for forbidden in [
        "prompt",
        "command",
        "patch",
        "tool_input",
        "tool_output",
        "transcript",
    ] {
        let mut value =
            serde_json::to_value(support::presence_record(PresenceStatus::Working)).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert(forbidden.into(), Value::String("private-canary".into()));
        assert!(serde_json::from_value::<devmap::presence::PresenceRecord>(value).is_err());
    }
}

#[test]
fn presence_schema_rejects_impossible_status_source_combinations() {
    let mut value =
        serde_json::to_value(support::presence_record(PresenceStatus::Completed)).unwrap();
    value["status_source"] = json!("lease");
    value["lease_expires_at"] = json!("2026-09-02T12:02:00Z");
    assert!(serde_json::from_value::<devmap::presence::PresenceRecord>(value).is_err());
}

#[test]
fn git_only_state_is_unknown_instead_of_invented() {
    let record = support::presence_record(PresenceStatus::Unknown);
    assert_eq!(record.status_source, StatusSource::GitOnly);
    assert_eq!(record.confidence, Confidence::Unknown);
}

fn observed_presence() -> (
    tempfile::TempDir,
    PresenceStore,
    devmap::presence::PresenceRecord,
    PathBuf,
) {
    let repo = support::committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let event = EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        "event-start",
        EventType::SessionStarted,
        1,
        "2026-09-02T12:00:00Z",
        HostIdentity::new("codex", "1.0.0").unwrap(),
        ActorIdentity::new("agent-main", None).unwrap(),
        SessionContext::new(
            "session-presence",
            Some("route-1".into()),
            workspace.root.to_string_lossy(),
            Some(workspace.root.to_string_lossy().into_owned()),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )
        .unwrap(),
        json!({"capture_grade": CaptureGrade::D}),
    )
    .unwrap();
    let journal_record = JournalStore::open(&workspace, "session-presence")
        .unwrap()
        .append(event)
        .unwrap();
    let store = PresenceStore::open(&workspace).unwrap();
    let now = OffsetDateTime::parse("2026-09-02T12:00:00Z", &Rfc3339).unwrap();
    let starting = store
        .observe(PresenceSignal::AcceptedRecords(&[journal_record]), now)
        .unwrap();
    assert_eq!(starting.status, PresenceStatus::Starting);
    assert_eq!(starting.route_id.as_deref(), Some("route-1"));
    let root = workspace.git_common_dir.join("devmap/presence/v1");
    (repo, store, starting, root)
}

fn presence_event(
    workspace: &devmap::git::SourceWorkspace,
    session_id: &str,
    event_id: &str,
    sequence: u64,
    event_type: EventType,
) -> EventEnvelope {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        event_id,
        event_type,
        sequence,
        "2026-09-02T12:00:00Z",
        HostIdentity::new("codex", "1.0.0").unwrap(),
        ActorIdentity::new("agent-main", None).unwrap(),
        SessionContext::new(
            session_id,
            None,
            workspace.root.to_string_lossy(),
            Some(workspace.root.to_string_lossy().into_owned()),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )
        .unwrap(),
        json!({"capture_grade": CaptureGrade::D}),
    )
    .unwrap()
}
