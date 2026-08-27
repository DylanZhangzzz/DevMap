use devmap::events::{
    ActorIdentity, CaptureCapabilities, CaptureGrade, EVENT_SCHEMA_VERSION, EventEnvelope,
    EventType, HostIdentity, SessionContext,
};
use serde_json::json;

fn valid_envelope(
    event_id: &str,
    sequence: u64,
    context: SessionContext,
    payload: serde_json::Value,
) -> Result<EventEnvelope, devmap::error::DevMapError> {
    EventEnvelope::new(
        EVENT_SCHEMA_VERSION,
        event_id,
        EventType::EvidenceRecorded,
        sequence,
        "2026-08-27T16:00:00Z",
        HostIdentity::new("codex", "1.0.0")?,
        ActorIdentity::new("agent-1", None)?,
        context,
        payload,
    )
}

fn valid_context() -> Result<SessionContext, devmap::error::DevMapError> {
    SessionContext::new(
        "session-1",
        Some("route-1".into()),
        "https://example.test/acme/devmap.git",
        Some("/workspace/devmap".into()),
        Some("main".into()),
        Some("0123456789abcdef0123456789abcdef01234567".into()),
    )
}

#[test]
fn event_envelope_rejects_invalid_required_values() {
    let cases: Vec<(&str, Result<EventEnvelope, devmap::error::DevMapError>)> = vec![
        (
            "blank event ID",
            valid_envelope("  ", 1, valid_context().unwrap(), json!({})),
        ),
        (
            "sequence zero",
            valid_envelope("evt-1", 0, valid_context().unwrap(), json!({})),
        ),
        (
            "unsupported schema version",
            EventEnvelope::new(
                "devmap/event/0",
                "evt-1",
                EventType::EvidenceRecorded,
                1,
                "2026-08-27T16:00:00Z",
                HostIdentity::new("codex", "1.0.0").unwrap(),
                ActorIdentity::new("agent-1", None).unwrap(),
                valid_context().unwrap(),
                json!({}),
            ),
        ),
        (
            "non-object payload",
            valid_envelope("evt-1", 1, valid_context().unwrap(), json!("text")),
        ),
        (
            "floating point payload",
            valid_envelope(
                "evt-1",
                1,
                valid_context().unwrap(),
                json!({"nested": [1.5]}),
            ),
        ),
    ];

    for (name, result) in cases {
        assert!(result.is_err(), "{name} must be rejected");
    }

    assert!(SessionContext::new("session-1", None, " ", None, None, None).is_err());
    assert!(SessionContext::new(" ", None, "repo", None, None, None).is_err());
    assert!(
        SessionContext::new(
            "session-1",
            None,
            "repo",
            None,
            None,
            Some("0123456789ABCDEF0123456789ABCDEF01234567".into()),
        )
        .is_err()
    );
}

#[test]
fn event_envelope_serializes_canonically_and_uses_snake_case_event_names() {
    let envelope = valid_envelope(
        "evt-1",
        1,
        valid_context().unwrap(),
        json!({"z": 1, "a": 2}),
    )
    .unwrap();

    let serialized = String::from_utf8(envelope.canonical_bytes().unwrap()).unwrap();
    assert!(serialized.contains("\"event_type\":\"evidence_recorded\""));
    assert_eq!(envelope.sha256().unwrap().len(), 64);
    assert_eq!(envelope.sha256().unwrap(), envelope.sha256().unwrap());
}

#[test]
fn capture_capability_matrix_assigns_expected_grades() {
    let native_lifecycle = vec![
        EventType::SessionStarted,
        EventType::SessionStopped,
        EventType::MutationObserved,
        EventType::EvidenceRecorded,
    ];
    let cases = [
        (
            "Codex native",
            CaptureCapabilities {
                lifecycle_events: native_lifecycle.clone(),
                pre_mutation_blocking: true,
                subagent_lifecycle: true,
                workspace_rebind: true,
                tool_results: true,
                commit_mapping: true,
                raw_transcript: false,
            },
            CaptureGrade::A,
        ),
        (
            "Claude native",
            CaptureCapabilities {
                lifecycle_events: native_lifecycle.clone(),
                pre_mutation_blocking: true,
                subagent_lifecycle: true,
                workspace_rebind: false,
                tool_results: true,
                commit_mapping: true,
                raw_transcript: false,
            },
            CaptureGrade::A,
        ),
        (
            "Generic MCP",
            CaptureCapabilities {
                lifecycle_events: vec![EventType::SessionStarted, EventType::MutationObserved],
                pre_mutation_blocking: false,
                subagent_lifecycle: false,
                workspace_rebind: false,
                tool_results: true,
                commit_mapping: true,
                raw_transcript: false,
            },
            CaptureGrade::C,
        ),
        (
            "Prompt only",
            CaptureCapabilities {
                lifecycle_events: vec![EventType::InstructionObserved],
                pre_mutation_blocking: false,
                subagent_lifecycle: false,
                workspace_rebind: false,
                tool_results: false,
                commit_mapping: false,
                raw_transcript: false,
            },
            CaptureGrade::D,
        ),
    ];

    for (name, capabilities, expected_grade) in cases {
        assert_eq!(capabilities.grade(), expected_grade, "{name}");
    }
}

#[test]
fn missing_tool_results_or_commit_mapping_never_reports_grade_a() {
    let lifecycle_events = vec![
        EventType::SessionStarted,
        EventType::SessionStopped,
        EventType::MutationObserved,
        EventType::EvidenceRecorded,
    ];

    for capabilities in [
        CaptureCapabilities {
            lifecycle_events: lifecycle_events.clone(),
            pre_mutation_blocking: true,
            subagent_lifecycle: true,
            workspace_rebind: true,
            tool_results: false,
            commit_mapping: true,
            raw_transcript: false,
        },
        CaptureCapabilities {
            lifecycle_events,
            pre_mutation_blocking: true,
            subagent_lifecycle: true,
            workspace_rebind: true,
            tool_results: true,
            commit_mapping: false,
            raw_transcript: false,
        },
    ] {
        assert_ne!(capabilities.grade(), CaptureGrade::A);
    }
}

#[test]
fn event_envelope_deserialization_applies_constructor_validation() {
    let raw = json!({
        "schema_version": EVENT_SCHEMA_VERSION,
        "event_id": "evt-1",
        "event_type": "evidence_recorded",
        "sequence": 1,
        "occurred_at": "2026-08-27T16:00:00Z",
        "host": {"name": "codex", "adapter_version": "1.0.0"},
        "actor": {"agent_id": "agent-1"},
        "context": {"session_id": "session-1", "repository": " "},
        "payload": {}
    });

    assert!(serde_json::from_value::<EventEnvelope>(raw).is_err());
}
