mod support;

use std::collections::{BTreeMap, BTreeSet};
use std::process::Command;

use devmap::dock::{DockReducer, DockService, NoRoutes};
use devmap::journal::{JournalIntegrity, JournalSummary, summarize_existing_sessions};
use devmap::presence::{Confidence, PresenceStatus, StatusSource};
use devmap::worktrees::repository_id;

#[test]
fn reducer_puts_current_first_and_unknown_worktrees_in_warning_group() {
    let fixture = support::dock_reducer_fixture();
    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();
    assert!(model.current[0].is_current);
    assert_eq!(model.active.len(), 1);
    assert_eq!(model.current[0].status, PresenceStatus::Unknown);
    assert_eq!(model.current[0].status_source, StatusSource::GitOnly);
    assert_eq!(model.current[0].confidence, Confidence::Unknown);
}

#[test]
fn reducer_preserves_multiple_agents_and_marks_corrupt_capture_incomplete() {
    let mut fixture = support::dock_reducer_fixture();
    let mut second = fixture.presence.records[0].clone();
    second.session_id = "waiting-session".into();
    second.actor_id = "agent-review".into();
    second.status = PresenceStatus::Waiting;
    second.status_source = StatusSource::HostExplicit;
    fixture.presence.records.push(second.clone());
    fixture.journals.insert(
        second.session_id.clone(),
        JournalSummary {
            session_id: second.session_id,
            records: 1,
            last_sequence: Some(1),
            last_sha256: Some("b".repeat(64)),
            integrity: JournalIntegrity::Corrupt,
        },
    );

    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();

    assert_eq!(model.active.len(), 2);
    assert_eq!(model.active[0].status, PresenceStatus::Waiting);
    assert!(model.active[0].capture_incomplete);
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "journal_corrupt")
    );
}

#[test]
fn reducer_orders_activity_by_instant_not_timestamp_spelling() {
    let mut fixture = support::dock_reducer_fixture();
    fixture.presence.records[0].session_id = "newer-session".into();
    fixture.presence.records[0].last_event_at = "2026-09-02T11:30:00Z".into();
    fixture.journals.clear();
    let mut older = fixture.presence.records[0].clone();
    older.session_id = "older-session".into();
    older.last_event_at = "2026-09-02T13:00:00+02:00".into();
    fixture.presence.records.push(older);
    for session_id in ["newer-session", "older-session"] {
        fixture.journals.insert(
            session_id.into(),
            JournalSummary {
                session_id: session_id.into(),
                records: 1,
                last_sequence: Some(1),
                last_sha256: Some("c".repeat(64)),
                integrity: JournalIntegrity::Verified,
            },
        );
    }

    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();
    assert_eq!(model.active[0].session_id.as_deref(), Some("newer-session"));
}

#[test]
fn reducer_skips_mismatched_presence_and_is_deterministic() {
    let mut fixture = support::dock_reducer_fixture();
    fixture.presence.records[0].repository_id = format!("sha256-{}", "f".repeat(64));
    fixture
        .presence
        .warnings
        .push(devmap::presence::PresenceWarning {
            code: "presence_record_invalid",
            subject_id: Some("corrupt-session".into()),
        });
    fixture.presence.truncated = true;
    let reducer = DockReducer::new(NoRoutes);
    let first = reducer
        .reduce(
            &fixture.workspace,
            fixture.worktrees.clone(),
            fixture.presence.clone(),
            fixture.journals.clone(),
            fixture.now,
        )
        .unwrap();
    let second = reducer
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();

    assert_eq!(first, second);
    assert!(first.truncated);
    assert_eq!(first.active.len(), 0);
    assert_eq!(first.stale_or_uninstrumented.len(), 1);
    assert!(
        first
            .warnings
            .iter()
            .any(|warning| { warning.code == "presence_repository_mismatch" })
    );
    assert!(first.warnings.iter().any(|warning| {
        warning.code == "presence_record_invalid"
            && warning.subject_id.as_deref() == Some("corrupt-session")
    }));
}

#[test]
fn journal_summary_is_read_only_and_reports_verified_missing_and_corrupt() {
    let repo = support::committed_repo();
    let workspace = devmap::git::SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let journal = devmap::journal::JournalStore::open(&workspace, "valid-session").unwrap();
    let event = devmap::events::EventEnvelope::new(
        devmap::events::EVENT_SCHEMA_VERSION,
        "summary-event",
        devmap::events::EventType::EvidenceRecorded,
        1,
        "2026-09-02T12:00:00Z",
        devmap::events::HostIdentity::new("test", "1").unwrap(),
        devmap::events::ActorIdentity::new("agent", None).unwrap(),
        devmap::events::SessionContext::new(
            "valid-session",
            None,
            workspace.root.to_string_lossy(),
            None,
            None,
            Some(workspace.head.clone()),
        )
        .unwrap(),
        serde_json::json!({}),
    )
    .unwrap();
    journal.append(event).unwrap();
    std::fs::create_dir_all(workspace.git_dir.join("devmap/sessions/corrupt-session")).unwrap();
    std::fs::write(
        workspace
            .git_dir
            .join("devmap/sessions/corrupt-session/events.ndjson"),
        b"not-json\n",
    )
    .unwrap();
    let before = support::source_snapshot(repo.path());

    let summaries = summarize_existing_sessions(
        &workspace,
        &BTreeSet::from([
            "valid-session".into(),
            "missing-session".into(),
            "corrupt-session".into(),
        ]),
    );

    assert_eq!(
        summaries["valid-session"].integrity,
        JournalIntegrity::Verified
    );
    assert_eq!(summaries["valid-session"].last_sequence, Some(1));
    assert_eq!(
        summaries["missing-session"].integrity,
        JournalIntegrity::Missing
    );
    assert_eq!(
        summaries["corrupt-session"].integrity,
        JournalIntegrity::Corrupt
    );
    assert_eq!(support::source_snapshot(repo.path()), before);
}

#[test]
fn journal_summary_finds_sessions_recorded_in_other_worktrees() {
    let repo = support::committed_repo();
    let linked = support::linked_worktree(repo.path(), "codex/summary-other");
    let main = devmap::git::SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    let other = devmap::git::SourceGitInspector::open(linked.path())
        .unwrap()
        .workspace()
        .unwrap();
    let event = devmap::events::EventEnvelope::new(
        devmap::events::EVENT_SCHEMA_VERSION,
        "other-summary-event",
        devmap::events::EventType::EvidenceRecorded,
        1,
        "2026-09-02T12:00:00Z",
        devmap::events::HostIdentity::new("test", "1").unwrap(),
        devmap::events::ActorIdentity::new("other-agent", None).unwrap(),
        devmap::events::SessionContext::new(
            "other-session",
            None,
            other.root.to_string_lossy(),
            None,
            other.branch.clone(),
            Some(other.head.clone()),
        )
        .unwrap(),
        serde_json::json!({}),
    )
    .unwrap();
    devmap::journal::JournalStore::open(&other, "other-session")
        .unwrap()
        .append(event)
        .unwrap();

    let summaries = summarize_existing_sessions(&main, &BTreeSet::from(["other-session".into()]));
    assert_eq!(
        summaries["other-session"].integrity,
        JournalIntegrity::Verified
    );
}

#[test]
fn dock_service_revision_changes_only_when_content_changes() {
    let repo = support::committed_repo();
    let mut service = DockService::open(repo.path()).unwrap();
    let first = service.snapshot().clone();
    let generated_at = time::OffsetDateTime::parse(
        &first.generated_at,
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    service.refresh(generated_at).unwrap();
    assert_eq!(service.snapshot().revision, first.revision);

    let _linked = support::linked_worktree(repo.path(), "codex/revision-change");
    service.refresh(generated_at).unwrap();
    assert_eq!(service.snapshot().revision, first.revision + 1);
    assert_ne!(
        service.snapshot().content_hash().unwrap(),
        first.content_hash().unwrap()
    );
}

#[test]
fn agents_json_is_canonical_bounded_and_does_not_change_source_git_state() {
    let repo = support::committed_repo();
    let before = support::source_snapshot(repo.path());
    let output = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args([
            "agents",
            "--source",
            repo.path().to_str().unwrap(),
            "--json",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output.stdout.len() < 1024 * 1024);
    let model: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(model["schema_version"], "devmap/dock/1");
    assert_eq!(
        model["repository_id"],
        repository_id(
            &devmap::git::SourceGitInspector::open(repo.path())
                .unwrap()
                .workspace()
                .unwrap()
        )
    );
    assert_eq!(support::source_snapshot(repo.path()), before);
}

#[test]
fn no_routes_never_fabricates_a_route() {
    let fixture = support::dock_reducer_fixture();
    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            BTreeMap::new(),
            fixture.now,
        )
        .unwrap();
    assert!(
        model
            .current
            .iter()
            .chain(&model.active)
            .chain(&model.stale_or_uninstrumented)
            .all(|entry| entry.route_id.is_none())
    );
}

#[test]
fn reducer_bounds_large_multi_agent_output_without_hiding_truncation() {
    let mut fixture = support::dock_reducer_fixture();
    let template = fixture.presence.records.remove(0);
    fixture.journals.clear();
    for index in 0..100 {
        let mut record = template.clone();
        record.session_id = format!("large-session-{index:03}");
        record.actor_id = format!("agent-{index:03}-{}", "x".repeat(16 * 1024 - 16));
        fixture.journals.insert(
            record.session_id.clone(),
            JournalSummary {
                session_id: record.session_id.clone(),
                records: 1,
                last_sequence: Some(1),
                last_sha256: Some("d".repeat(64)),
                integrity: JournalIntegrity::Verified,
            },
        );
        fixture.presence.records.push(record);
    }

    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            fixture.worktrees,
            fixture.presence,
            fixture.journals,
            fixture.now,
        )
        .unwrap();
    let bytes = devmap::canonical::canonical_json(&model).unwrap();
    assert!(model.truncated);
    assert!(bytes.len() <= devmap::dock::MAX_DOCK_MODEL_BYTES);
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "dock_output_truncated")
    );
}
