mod support;

use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

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
        "session_id": "session-1",
        "cwd": "/workspace/devmap",
        "hook_event_name": "UserPromptSubmit",
        "prompt": "Keep the API backwards compatible.",
        "tool_name": "Bash",
        "tool_input": {"path": "src/lib.rs"},
        "vendor_trace": "opaque host detail"
    })
}

fn claude_input() -> Value {
    json!({
        "session_id": "session-1",
        "cwd": "/workspace/devmap",
        "hook_event_name": "UserPromptSubmit",
        "prompt": "Keep the API backwards compatible.",
        "tool_name": "Bash",
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
        assert_eq!(codex[0].actor().agent_id(), "codex:session-1");
        assert_eq!(claude[0].actor().agent_id(), "claude:session-1");
        assert!(!codex[0].occurred_at().is_empty());
        assert!(!claude[0].occurred_at().is_empty());
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
fn native_human_prompt_becomes_an_instruction_reference_without_persisting_transcript() {
    let (_repo, workspace) = workspace();
    let events = normalize_hook_input(
        AdapterHost::Codex,
        "UserPromptSubmit",
        codex_input(),
        &workspace,
    )
    .unwrap();

    assert_eq!(events[0].event_type(), &EventType::InstructionObserved);
    assert_eq!(
        events[0].payload()["requirement_trace"]["content_stored"],
        false
    );
    assert!(
        events[0].payload()["requirement_trace"]["content_digest"]
            .as_str()
            .unwrap()
            .starts_with("sha256-")
    );
    assert!(
        events[0].payload()["requirement_trace"]
            .get("approved_quotation")
            .is_none()
    );
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
        serde_json::from_str::<Value>(&output.stdout).unwrap(),
        json!({})
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
    assert_eq!(payload["requirement_trace"]["content_stored"], false);
    assert!(
        payload["requirement_trace"]
            .get("approved_quotation")
            .is_none()
    );
    assert!(payload.get("prompt").is_none());
    assert!(payload["host_metadata"].get("prompt").is_none());
}

#[test]
fn supplied_subagent_identity_and_parent_are_preserved() {
    let (_repo, workspace) = workspace();
    let mut input = codex_input();
    input.as_object_mut().unwrap().extend([
        ("agent_id".into(), Value::String("subagent-7".into())),
        ("parent_agent_id".into(), Value::String("main-agent".into())),
    ]);

    let event = normalize_hook_input(AdapterHost::Codex, "SubagentStart", input, &workspace)
        .unwrap()
        .remove(0);

    assert_eq!(event.actor().agent_id(), "subagent-7");
    assert_eq!(event.actor().parent_agent_id(), Some("main-agent"));
}

#[test]
fn shell_tools_are_treated_as_write_capable_after_execution() {
    let (_repo, workspace) = workspace();
    for tool_name in ["Bash", "PowerShell", "shell", "exec"] {
        let mut input = codex_input();
        input
            .as_object_mut()
            .unwrap()
            .insert("tool_name".into(), Value::String(tool_name.into()));
        let event_types: Vec<_> =
            normalize_hook_input(AdapterHost::Codex, "PostToolUse", input, &workspace)
                .unwrap()
                .into_iter()
                .map(|event| event.event_type().clone())
                .collect();
        assert_eq!(
            event_types,
            vec![
                EventType::ToolCompleted,
                EventType::MutationObserved,
                EventType::CaptureGap,
            ],
            "{tool_name}"
        );
    }
}

#[test]
fn metadata_redacts_sensitive_fields_at_every_nesting_level_and_survives_persistence() {
    let (repo, workspace) = workspace();
    let mut input = codex_input();
    input.as_object_mut().unwrap().insert(
        "vendor_trace".into(),
        json!({
            "safe": "keep",
            "tool_response": "remove",
            "nested": {"compact_summary": "remove", "message": "remove", "safe": "keep"}
        }),
    );
    let mut stdin = Cursor::new(serde_json::to_vec(&input).unwrap());
    handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "FutureLifecycleEvent".into(),
        },
        &mut stdin,
    )
    .unwrap();

    let records = JournalStore::open(&workspace, "session-1")
        .unwrap()
        .replay()
        .unwrap();
    let metadata = &records[0].event.payload()["host_metadata"];
    assert_eq!(metadata["vendor_trace"]["safe"], "keep");
    assert_eq!(metadata["vendor_trace"]["nested"]["safe"], "keep");
    assert!(metadata["vendor_trace"].get("tool_response").is_none());
    assert!(
        metadata["vendor_trace"]["nested"]
            .get("compact_summary")
            .is_none()
    );
    assert!(metadata["vendor_trace"]["nested"].get("message").is_none());
}

#[test]
fn unsupported_event_wins_over_missing_native_context() {
    let (_repo, workspace) = workspace();
    let events = normalize_hook_input(
        AdapterHost::Codex,
        "FutureLifecycleEvent",
        json!({}),
        &workspace,
    )
    .unwrap();

    assert_eq!(events[0].event_type(), &EventType::CaptureGap);
    assert_eq!(events[0].payload()["reason"], "unsupported_host_event");
}

#[test]
fn metadata_string_truncation_respects_utf8_byte_cap() {
    let (_repo, workspace) = workspace();
    let mut input = codex_input();
    input
        .as_object_mut()
        .unwrap()
        .insert("vendor_trace".into(), Value::String("€".repeat(600)));
    let event = normalize_hook_input(AdapterHost::Codex, "PreToolUse", input, &workspace)
        .unwrap()
        .remove(0);
    assert!(
        event.payload()["host_metadata"]["vendor_trace"]
            .as_str()
            .unwrap()
            .len()
            <= 1024
    );
}

#[test]
fn concurrent_hook_processes_append_a_complete_same_session_journal() {
    let (repo, workspace) = workspace();
    let executable = std::env::var("CARGO_BIN_EXE_devmap").expect("test binary path");
    let mut children = Vec::new();
    for event_id in ["concurrent-1", "concurrent-2"] {
        let mut child = Command::new(&executable)
            .args([
                "hook",
                "handle",
                "--host",
                "codex",
                "--event",
                "SessionStart",
                "--source",
                repo.path().to_str().unwrap(),
            ])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut input = codex_input();
        input
            .as_object_mut()
            .unwrap()
            .insert("event_id".into(), Value::String(event_id.into()));
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&input).unwrap())
            .unwrap();
        children.push(child);
    }

    for child in children {
        assert!(child.wait_with_output().unwrap().status.success());
    }
    let records = JournalStore::open(&workspace, "session-1")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].sequence, 1);
    assert_eq!(records[1].sequence, 2);
}
