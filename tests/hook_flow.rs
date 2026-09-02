mod support;

use std::io::Cursor;

use devmap::cli::{AdapterHost, HookHandleArgs};
use devmap::events::EventType;
use devmap::git::SourceGitInspector;
use devmap::hook::{handle_hook, normalize_hook_input};
use devmap::journal::JournalStore;
use serde_json::{Value, json};
use support::committed_repo;

fn workspace() -> (tempfile::TempDir, devmap::git::SourceWorkspace) {
    let repo = committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    (repo, workspace)
}

fn codex_input() -> Value {
    json!({
        "event_id": "codex-event-1",
        "session_id": "session-1",
        "agent_id": "agent-1",
        "parent_agent_id": "parent-1",
        "timestamp": "2026-08-27T16:00:00Z",
        "approved_quotation": "Keep the API backwards compatible.",
        "source_locator": "turn:7",
        "tool_name": "write_file",
        "tool_input": {"path": "src/lib.rs"},
        "tool_result": {"status": "ok"},
        "vendor_trace": "opaque host detail"
    })
}

fn claude_input() -> Value {
    json!({
        "hook_event_id": "claude-event-1",
        "sessionId": "session-1",
        "agentId": "agent-1",
        "parentAgentId": "parent-1",
        "occurred_at": "2026-08-27T16:00:00Z",
        "approved_quotation": "Keep the API backwards compatible.",
        "source_locator": "turn:7",
        "tool_name": "Write",
        "tool_input": {"path": "src/lib.rs"},
        "tool_response": {"status": "ok"},
        "vendor_trace": "opaque host detail"
    })
}

#[test]
fn codex_and_claude_lifecycle_fixtures_normalize_to_equivalent_event_types() {
    let (_repo, workspace) = workspace();
    let expected = [
        ("SessionStart", vec![EventType::SessionStarted]),
        ("UserPromptSubmit", vec![EventType::InstructionObserved]),
        ("PreToolUse", vec![EventType::ToolRequested]),
        (
            "PostToolUse",
            vec![
                EventType::ToolCompleted,
                EventType::MutationObserved,
                EventType::CaptureGap,
            ],
        ),
        ("PreCompact", vec![EventType::ContextCompacting]),
        ("PostCompact", vec![EventType::ContextCompacted]),
        ("SubagentStart", vec![EventType::AgentStarted]),
        ("SubagentStop", vec![EventType::AgentStopped]),
        ("Stop", vec![EventType::SessionStopped]),
        ("SessionEnd", vec![EventType::SessionStopped]),
    ];

    for (event, event_types) in expected {
        let codex =
            normalize_hook_input(AdapterHost::Codex, event, codex_input(), &workspace).unwrap();
        let claude =
            normalize_hook_input(AdapterHost::Claude, event, claude_input(), &workspace).unwrap();
        let codex_types: Vec<_> = codex
            .iter()
            .map(|entry| entry.event_type().clone())
            .collect();
        let claude_types: Vec<_> = claude
            .iter()
            .map(|entry| entry.event_type().clone())
            .collect();

        assert_eq!(codex_types, event_types, "Codex {event}");
        assert_eq!(claude_types, event_types, "Claude {event}");
        assert_eq!(codex_types, claude_types, "equivalent {event} hooks");
        assert_eq!(
            codex[0].actor().parent_agent_id(),
            Some("parent-1"),
            "Codex {event} must preserve parentage"
        );
        assert_eq!(
            claude[0].actor().parent_agent_id(),
            Some("parent-1"),
            "Claude {event} must preserve parentage"
        );
    }
}

#[test]
fn normalization_retains_unknown_fields_only_in_bounded_host_metadata() {
    let (_repo, workspace) = workspace();
    let event = normalize_hook_input(AdapterHost::Codex, "PreToolUse", codex_input(), &workspace)
        .unwrap()
        .remove(0);

    assert_eq!(
        event.payload()["host_metadata"]["vendor_trace"],
        "opaque host detail"
    );
    assert!(event.payload().get("vendor_trace").is_none());
    assert!(event.payload()["host_metadata"].as_object().unwrap().len() <= 16);
}

#[test]
fn missing_mandatory_context_is_a_capture_gap_instead_of_a_crash() {
    let (_repo, workspace) = workspace();
    let mut input = codex_input();
    input.as_object_mut().unwrap().remove("session_id");

    let events =
        normalize_hook_input(AdapterHost::Codex, "SessionStart", input, &workspace).unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type(), &EventType::CaptureGap);
    assert_eq!(events[0].payload()["reason"], "missing_mandatory_context");
}

#[test]
fn unstructured_human_prompt_becomes_a_gap_without_capturing_its_transcript() {
    let (_repo, workspace) = workspace();
    let mut input = codex_input();
    let input = input.as_object_mut().unwrap();
    input.remove("approved_quotation");
    input.insert("prompt".into(), Value::String("raw host transcript".into()));

    let events = normalize_hook_input(
        AdapterHost::Codex,
        "UserPromptSubmit",
        Value::Object(input.clone()),
        &workspace,
    )
    .unwrap();

    assert_eq!(events[0].event_type(), &EventType::CaptureGap);
    assert_eq!(events[0].payload()["reason"], "missing_mandatory_context");
    assert!(events[0].payload()["host_metadata"].get("prompt").is_none());
}

#[test]
fn handler_returns_continue_for_unsupported_event_and_persists_one_gap() {
    let (repo, workspace) = workspace();
    let mut stdin = Cursor::new(serde_json::to_vec(&codex_input()).unwrap());
    let output = handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "FutureLifecycleEvent".into(),
        },
        &mut stdin,
    )
    .unwrap();

    assert_eq!(output.exit_code, 0);
    assert_eq!(
        serde_json::from_str::<Value>(&output.stdout).unwrap()["continue"],
        true
    );
    let records = JournalStore::open(&workspace, "session-1")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event.event_type(), &EventType::CaptureGap);
    assert_eq!(
        records[0].event.payload()["reason"],
        "unsupported_host_event"
    );
}

#[test]
fn malformed_json_is_a_typed_error_and_writes_no_journal_entry() {
    let (repo, workspace) = workspace();
    let mut stdin = Cursor::new(br#"{\"session_id\": "#);
    let error = handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "SessionStart".into(),
        },
        &mut stdin,
    )
    .unwrap_err();

    assert!(matches!(error, devmap::error::DevMapError::Json(_)));
    assert!(
        JournalStore::open(&workspace, "session-1")
            .unwrap()
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn human_prompt_normalization_never_persists_raw_transcript() {
    let (repo, workspace) = workspace();
    let mut input = codex_input();
    input.as_object_mut().unwrap().insert(
        "prompt".into(),
        Value::String("this raw transcript must never be stored".into()),
    );
    let mut stdin = Cursor::new(serde_json::to_vec(&input).unwrap());

    handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "UserPromptSubmit".into(),
        },
        &mut stdin,
    )
    .unwrap();

    let records = JournalStore::open(&workspace, "session-1")
        .unwrap()
        .replay()
        .unwrap();
    let payload = records[0].event.payload();
    assert_eq!(
        payload["requirement_trace"]["approved_quotation"],
        "Keep the API backwards compatible."
    );
    assert!(payload.get("prompt").is_none());
    assert!(payload["host_metadata"].get("prompt").is_none());
}
