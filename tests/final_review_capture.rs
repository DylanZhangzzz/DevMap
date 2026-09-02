mod support;

use std::io::Cursor;

use devmap::capture::{
    AgentDecisionInput, CaptureKernel, MAX_CAPTURE_LIST_ITEMS, MAX_CAPTURE_STRING_BYTES,
    RequirementTraceInput,
};
use devmap::cli::{AdapterHost, HookHandleArgs};
use devmap::events::{
    ActorIdentity, CaptureGrade, EventType, HostIdentity, SessionContext, host_capabilities,
};
use devmap::git::SourceGitInspector;
use devmap::hook::{MAX_HOOK_BODY_BYTES, handle_hook, normalize_hook_input};
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

fn official_codex(event: &str) -> Value {
    let mut value = json!({
        "session_id": "thr_123",
        "transcript_path": "/workspace/.codex/rollout.jsonl",
        "cwd": "/workspace",
        "hook_event_name": event,
        "model": "gpt-5.6-sol"
    });
    let object = value.as_object_mut().unwrap();
    match event {
        "SessionStart" => {
            object.insert("source".into(), json!("startup"));
            object.insert("permission_mode".into(), json!("default"));
        }
        "UserPromptSubmit" => {
            object.insert("turn_id".into(), json!("turn_123"));
            object.insert("permission_mode".into(), json!("default"));
            object.insert("prompt".into(), json!("CANARY_PROMPT_SECRET"));
        }
        "PreToolUse" => {
            object.insert("turn_id".into(), json!("turn_123"));
            object.insert("permission_mode".into(), json!("default"));
            object.insert("tool_name".into(), json!("Bash"));
            object.insert("tool_use_id".into(), json!("tool_123"));
            object.insert(
                "tool_input".into(),
                json!({"command": "CANARY_COMMAND_SECRET"}),
            );
        }
        "PostToolUse" => {
            object.insert("turn_id".into(), json!("turn_123"));
            object.insert("permission_mode".into(), json!("default"));
            object.insert("tool_name".into(), json!("Bash"));
            object.insert("tool_use_id".into(), json!("tool_123"));
            object.insert(
                "tool_input".into(),
                json!({"command": "CANARY_COMMAND_SECRET"}),
            );
            object.insert("tool_response".into(), json!("CANARY_OUTPUT_SECRET"));
        }
        "SubagentStart" => {
            object.insert("turn_id".into(), json!("turn_123"));
            object.insert("permission_mode".into(), json!("default"));
            object.insert("agent_id".into(), json!("agent_123"));
            object.insert("agent_type".into(), json!("explorer"));
        }
        "Stop" => {
            object.insert("turn_id".into(), json!("turn_123"));
            object.insert("permission_mode".into(), json!("default"));
            object.insert("stop_hook_active".into(), json!(false));
            object.insert(
                "last_assistant_message".into(),
                json!("CANARY_OUTPUT_SECRET"),
            );
        }
        "SessionEnd" => {
            object.insert("reason".into(), json!("other"));
        }
        _ => unreachable!(),
    }
    value
}

#[test]
fn effective_host_capabilities_are_derived_and_honestly_grade_d() {
    for host in [
        AdapterHost::Codex,
        AdapterHost::Claude,
        AdapterHost::GenericMcp,
    ] {
        let capabilities = host_capabilities(host);
        assert_eq!(capabilities.grade(), CaptureGrade::D, "{host:?}");
        assert!(!capabilities.pre_mutation_blocking);
        assert!(!capabilities.workspace_rebind);
        assert!(!capabilities.tool_results);
        assert!(!capabilities.commit_mapping);
        assert!(
            !capabilities
                .lifecycle_events
                .contains(&EventType::MutationObserved)
        );
        if host != AdapterHost::GenericMcp {
            assert!(
                !capabilities
                    .lifecycle_events
                    .contains(&EventType::EvidenceRecorded)
            );
        }
    }
}

#[test]
fn stop_is_turn_completion_and_session_end_alone_stops_the_session() {
    let (_repo, workspace) = workspace();
    let stop = normalize_hook_input(
        AdapterHost::Codex,
        "Stop",
        official_codex("Stop"),
        &workspace,
    )
    .unwrap();
    let end = normalize_hook_input(
        AdapterHost::Codex,
        "SessionEnd",
        official_codex("SessionEnd"),
        &workspace,
    )
    .unwrap();

    assert_eq!(stop[0].event_type(), &EventType::TurnCompleted);
    assert_eq!(end[0].event_type(), &EventType::SessionStopped);
}

#[test]
fn official_subagent_payload_derives_the_main_actor_parent() {
    let (_repo, workspace) = workspace();
    let event = normalize_hook_input(
        AdapterHost::Codex,
        "SubagentStart",
        official_codex("SubagentStart"),
        &workspace,
    )
    .unwrap()
    .remove(0);

    assert_eq!(event.actor().agent_id(), "agent_123");
    assert_eq!(event.actor().parent_agent_id(), Some("codex:thr_123"));
}

#[test]
fn completed_write_capable_tool_is_activity_plus_an_unverified_mutation_gap() {
    let (_repo, workspace) = workspace();
    let events = normalize_hook_input(
        AdapterHost::Codex,
        "PostToolUse",
        official_codex("PostToolUse"),
        &workspace,
    )
    .unwrap();

    assert_eq!(
        events
            .iter()
            .map(|event| event.event_type())
            .collect::<Vec<_>>(),
        vec![&EventType::ToolCompleted, &EventType::CaptureGap]
    );
    assert_eq!(events[1].payload()["reason"], "mutation_unverified");
    assert!(events[1].payload().get("mutation_target").is_none());
}

#[test]
fn identifiable_retried_hooks_are_idempotent_and_event_name_mismatch_is_a_gap() {
    let (repo, workspace) = workspace();
    let input = official_codex("PostToolUse");
    for _ in 0..2 {
        handle_hook(
            HookHandleArgs {
                source: repo.path().to_path_buf(),
                host: AdapterHost::Codex,
                event: "PostToolUse".into(),
            },
            &mut Cursor::new(serde_json::to_vec(&input).unwrap()),
        )
        .unwrap();
    }
    let records = JournalStore::open(&workspace, "thr_123")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(
        records.len(),
        2,
        "the retried two-record hook is not duplicated"
    );

    let mismatch = normalize_hook_input(
        AdapterHost::Codex,
        "PreToolUse",
        official_codex("PostToolUse"),
        &workspace,
    )
    .unwrap();
    assert_eq!(mismatch.len(), 1);
    assert_eq!(mismatch[0].event_type(), &EventType::CaptureGap);
    assert_eq!(mismatch[0].payload()["reason"], "host_event_mismatch");
}

#[test]
fn status_values_do_not_collide_and_explicit_event_ids_are_stable() {
    let (_repo, workspace) = workspace();
    let normalize = |event_id: Option<&str>, sequence: u64| {
        let mut input = official_codex("SessionEnd");
        let object = input.as_object_mut().unwrap();
        object.insert("sequence".into(), json!(sequence));
        if let Some(event_id) = event_id {
            object.insert("event_id".into(), json!(event_id));
        }
        normalize_hook_input(AdapterHost::Codex, "SessionEnd", input, &workspace)
            .unwrap()
            .remove(0)
    };

    let first_unidentified = normalize(None, 1);
    let second_unidentified = normalize(None, 2);
    assert_ne!(
        first_unidentified.event_id(),
        second_unidentified.event_id(),
        "a repeated status reason is not a unique event identifier"
    );

    let identified = normalize(Some("session-close-1"), 3);
    let identified_retry = normalize(Some("session-close-1"), 4);
    let different_event = normalize(Some("session-close-2"), 5);
    assert_eq!(identified.event_id(), identified_retry.event_id());
    assert_ne!(identified.event_id(), different_event.event_id());
}

#[test]
fn native_persistence_uses_an_allowlist_and_has_a_total_byte_bound() {
    let (_repo, workspace) = workspace();
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "SubagentStart",
        "Stop",
        "SessionEnd",
    ] {
        let mut input = official_codex(event);
        input.as_object_mut().unwrap().insert(
            "unknown_nested".into(),
            json!({"innocent_name": "CANARY_NESTED_SECRET"}),
        );
        let events = normalize_hook_input(AdapterHost::Codex, event, input, &workspace).unwrap();
        for normalized in events {
            let bytes = normalized.canonical_bytes().unwrap();
            let text = String::from_utf8(bytes.clone()).unwrap();
            assert!(bytes.len() <= 64 * 1024);
            for secret in [
                "CANARY_PROMPT_SECRET",
                "CANARY_COMMAND_SECRET",
                "CANARY_OUTPUT_SECRET",
                "CANARY_NESTED_SECRET",
                "/workspace/.codex/rollout.jsonl",
            ] {
                assert!(!text.contains(secret), "{event} persisted {secret}");
            }
            assert!(normalized.payload().get("unknown_nested").is_none());
            assert!(normalized.payload().get("host_metadata").is_none());
        }
    }
}

#[test]
fn oversized_hook_input_is_rejected_before_json_parsing() {
    let (repo, _workspace) = workspace();
    let mut bytes = vec![b' '; MAX_HOOK_BODY_BYTES + 1];
    bytes[0] = b'{';
    let error = handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "SessionStart".into(),
        },
        &mut Cursor::new(bytes),
    )
    .unwrap_err();
    assert!(matches!(
        error,
        devmap::error::DevMapError::ResourceLimit { .. }
    ));
}

#[test]
fn invalid_native_timestamp_falls_back_to_a_valid_receipt_time() {
    let (_repo, workspace) = workspace();
    let mut input = official_codex("SessionStart");
    input
        .as_object_mut()
        .unwrap()
        .insert("timestamp".into(), json!("not-rfc3339"));
    let event = normalize_hook_input(AdapterHost::Codex, "SessionStart", input, &workspace)
        .unwrap()
        .remove(0);
    assert_ne!(event.occurred_at(), "not-rfc3339");
    assert!(
        time::OffsetDateTime::parse(
            event.occurred_at(),
            &time::format_description::well_known::Rfc3339
        )
        .is_ok()
    );
}

#[test]
fn capture_kernel_rejects_a_journal_for_another_session() {
    let (_repo, workspace) = workspace();
    let journal = JournalStore::open(&workspace, "journal-session").unwrap();
    let error = CaptureKernel::new(
        journal,
        host_capabilities(AdapterHost::GenericMcp),
        HostIdentity::new("generic_mcp", "devmap-mcp/1").unwrap(),
        ActorIdentity::new("agent-1", None).unwrap(),
        SessionContext::new(
            "other-session",
            None,
            workspace.root.to_string_lossy(),
            Some(workspace.root.to_string_lossy().into_owned()),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )
        .unwrap(),
    )
    .unwrap_err();
    assert!(error.to_string().contains("session"));
}

#[test]
fn context_head_must_be_a_lowercase_sha1_or_sha256() {
    for invalid in [
        "main",
        "abc123",
        "0123456789abcdef0123456789abcdef0123456g",
        "0123456789ABCDEF0123456789ABCDEF01234567",
    ] {
        assert!(
            SessionContext::new("session", None, "repo", None, None, Some(invalid.into())).is_err(),
            "accepted {invalid}"
        );
    }
}

#[test]
fn capture_kernel_bounds_semantic_strings_and_arrays_even_outside_mcp() {
    let (_repo, workspace) = workspace();
    let kernel = CaptureKernel::new(
        JournalStore::open(&workspace, "bounded-kernel").unwrap(),
        host_capabilities(AdapterHost::GenericMcp),
        HostIdentity::new("generic_mcp", "devmap-mcp/1").unwrap(),
        ActorIdentity::new("agent-1", None).unwrap(),
        SessionContext::new(
            "bounded-kernel",
            None,
            workspace.root.to_string_lossy(),
            Some(workspace.root.to_string_lossy().into_owned()),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )
        .unwrap(),
    )
    .unwrap();

    let long = kernel
        .record_requirement(
            "event-long",
            "2026-08-27T00:00:00Z",
            RequirementTraceInput {
                source_kind: "human_instruction".into(),
                source_locator: None,
                quoted_text: "x".repeat(MAX_CAPTURE_STRING_BYTES + 1),
            },
            false,
        )
        .unwrap_err();
    let many = kernel
        .record_decision(
            "event-many",
            "2026-08-27T00:00:00Z",
            AgentDecisionInput {
                decision: "Bound lists.".into(),
                basis: vec!["basis".into(); MAX_CAPTURE_LIST_ITEMS + 1],
                alternatives: vec!["one".into()],
                rationale: "bounded".into(),
                scope: "kernel".into(),
                authority: "review".into(),
                revisit_trigger: "new schema".into(),
            },
        )
        .unwrap_err();

    assert!(matches!(
        long,
        devmap::error::DevMapError::ResourceLimit { .. }
    ));
    assert!(matches!(
        many,
        devmap::error::DevMapError::ResourceLimit { .. }
    ));
    assert!(
        JournalStore::open(&workspace, "bounded-kernel")
            .unwrap()
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn event_envelope_limit_applies_to_the_total_serialized_event_not_only_payload() {
    let error = devmap::events::EventEnvelope::new(
        devmap::events::EVENT_SCHEMA_VERSION,
        "oversized-context-event",
        EventType::CaptureGap,
        1,
        "2026-08-27T00:00:00Z",
        HostIdentity::new("test-host", "1").unwrap(),
        ActorIdentity::new("agent-1", None).unwrap(),
        SessionContext::new(
            "session",
            None,
            "r".repeat(devmap::events::MAX_EVENT_BYTES),
            None,
            None,
            None,
        )
        .unwrap(),
        json!({"capture_grade": "D"}),
    )
    .unwrap_err();

    assert!(matches!(
        error,
        devmap::error::DevMapError::ResourceLimit { .. }
    ));
}
