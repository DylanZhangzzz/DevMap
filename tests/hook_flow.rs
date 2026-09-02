mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Write};
use std::process::{Command, Stdio};

use devmap::cli::{AdapterHost, HookHandleArgs};
use devmap::events::EventType;
use devmap::git::SourceGitInspector;
use devmap::hook::{handle_hook, normalize_hook_input};
use devmap::journal::JournalStore;
use devmap::presence::{PresenceStatus, PresenceStore};
use serde_json::{Value, json};
use support::committed_repo;

const EVENTS: [&str; 10] = [
    "SessionStart",
    "UserPromptSubmit",
    "PreToolUse",
    "PostToolUse",
    "PreCompact",
    "PostCompact",
    "SubagentStart",
    "SubagentStop",
    "Stop",
    "SessionEnd",
];

fn workspace() -> (tempfile::TempDir, devmap::git::SourceWorkspace) {
    let repo = committed_repo();
    let workspace = SourceGitInspector::open(repo.path())
        .unwrap()
        .workspace()
        .unwrap();
    (repo, workspace)
}

fn fixture(host: AdapterHost, event: &str) -> Value {
    let raw = match host {
        AdapterHost::Codex => include_str!("fixtures/hooks/codex-events.json"),
        AdapterHost::Claude => include_str!("fixtures/hooks/claude-events.json"),
        AdapterHost::GenericMcp => unreachable!(),
    };
    serde_json::from_str::<BTreeMap<String, Value>>(raw).unwrap()[event].clone()
}

#[test]
fn pinned_official_event_fixtures_have_the_same_truthful_lifecycle_contract() {
    let (_repo, workspace) = workspace();
    let expected = [
        vec![EventType::SessionStarted],
        vec![EventType::InstructionObserved],
        vec![EventType::ToolRequested],
        vec![EventType::ToolCompleted, EventType::CaptureGap],
        vec![EventType::ContextCompacting],
        vec![EventType::ContextCompacted],
        vec![EventType::AgentStarted],
        vec![EventType::AgentStopped],
        vec![EventType::TurnCompleted],
        vec![EventType::SessionStopped],
    ];

    for host in [AdapterHost::Codex, AdapterHost::Claude] {
        for (event, expected_types) in EVENTS.into_iter().zip(&expected) {
            let actual = normalize_hook_input(host, event, fixture(host, event), &workspace)
                .unwrap()
                .into_iter()
                .map(|entry| entry.event_type().clone())
                .collect::<Vec<_>>();
            assert_eq!(&actual, expected_types, "{host:?} {event}");
        }
    }
}

#[test]
fn native_content_and_unknown_recursive_metadata_never_cross_the_allowlist() {
    let (_repo, workspace) = workspace();
    let canaries = [
        "CANARY_TRANSCRIPT_SECRET",
        "CANARY_CWD_SECRET",
        "CANARY_PROMPT_SECRET",
        "CANARY_COMMAND_SECRET",
        "CANARY_OUTPUT_SECRET",
        "CANARY_COMPACT_SECRET",
        "CANARY_ASSISTANT_SECRET",
        "CANARY_UNKNOWN_SECRET",
    ];
    for host in [AdapterHost::Codex, AdapterHost::Claude] {
        for event in EVENTS {
            let mut input = fixture(host, event);
            input["transcript_path"] = json!("CANARY_TRANSCRIPT_SECRET");
            input["cwd"] = json!("CANARY_CWD_SECRET");
            input["unknown_nested"] = json!({"safe-looking": "CANARY_UNKNOWN_SECRET"});
            let normalized = normalize_hook_input(host, event, input, &workspace).unwrap();
            for entry in normalized {
                let persisted = String::from_utf8(entry.canonical_bytes().unwrap()).unwrap();
                for canary in canaries {
                    assert!(
                        !persisted.contains(canary),
                        "{host:?} {event} leaked {canary}"
                    );
                }
                assert!(entry.payload().get("host_metadata").is_none());
                assert!(entry.payload().get("unknown_nested").is_none());
            }
        }
    }
}

#[test]
fn an_unrecognized_session_end_reason_is_not_persisted_as_free_form_status() {
    let (_repo, workspace) = workspace();
    let mut input = fixture(AdapterHost::Claude, "SessionEnd");
    input["reason"] = json!("CANARY_REASON_SECRET");
    let event = normalize_hook_input(AdapterHost::Claude, "SessionEnd", input, &workspace)
        .unwrap()
        .remove(0);
    let persisted = String::from_utf8(event.canonical_bytes().unwrap()).unwrap();
    assert!(!persisted.contains("CANARY_REASON_SECRET"));
    assert!(event.payload()["status"].get("reason").is_none());
}

#[test]
fn invalid_identifier_shaped_native_fields_do_not_cross_the_allowlist() {
    let (_repo, workspace) = workspace();
    let mut mismatched = fixture(AdapterHost::Codex, "SessionStart");
    mismatched["hook_event_name"] = json!("CANARY EVENT NAME SECRET");
    let mismatch = normalize_hook_input(AdapterHost::Codex, "SessionStart", mismatched, &workspace)
        .unwrap()
        .remove(0);

    let mut tool = fixture(AdapterHost::Codex, "PostToolUse");
    tool["tool_name"] = json!("CANARY TOOL NAME SECRET");
    let tool = normalize_hook_input(AdapterHost::Codex, "PostToolUse", tool, &workspace)
        .unwrap()
        .remove(0);

    let persisted = format!(
        "{}{}",
        String::from_utf8(mismatch.canonical_bytes().unwrap()).unwrap(),
        String::from_utf8(tool.canonical_bytes().unwrap()).unwrap()
    );
    assert!(!persisted.contains("CANARY EVENT NAME SECRET"));
    assert!(!persisted.contains("CANARY TOOL NAME SECRET"));
    assert_eq!(mismatch.event_type(), &EventType::CaptureGap);
    assert_eq!(tool.payload()["tool"]["name"], "unknown");
}

#[test]
fn documented_session_end_reason_is_preserved_as_bounded_status() {
    let (_repo, workspace) = workspace();
    let mut input = fixture(AdapterHost::Claude, "SessionEnd");
    input["reason"] = json!("bypass_permissions_disabled");
    let event = normalize_hook_input(AdapterHost::Claude, "SessionEnd", input, &workspace)
        .unwrap()
        .remove(0);

    assert_eq!(
        event.payload()["status"]["reason"],
        "bypass_permissions_disabled"
    );
}

#[test]
fn missing_official_session_id_is_a_gap_and_thread_id_is_not_a_guessed_alias() {
    let (_repo, workspace) = workspace();
    let events = normalize_hook_input(
        AdapterHost::Codex,
        "SessionStart",
        json!({
            "thread_id": "synthetic-thread-alias",
            "hook_event_name": "SessionStart",
            "source": "startup"
        }),
        &workspace,
    )
    .unwrap();

    assert_eq!(events.len(), 1);
    assert_eq!(events[0].event_type(), &EventType::CaptureGap);
    assert_eq!(events[0].payload()["reason"], "missing_mandatory_context");
    assert_eq!(events[0].context().session_id(), "missing-session");
}

#[test]
fn native_prompt_is_activity_with_a_digest_not_semantic_requirement_evidence() {
    let (_repo, workspace) = workspace();
    let event = normalize_hook_input(
        AdapterHost::Codex,
        "UserPromptSubmit",
        fixture(AdapterHost::Codex, "UserPromptSubmit"),
        &workspace,
    )
    .unwrap()
    .remove(0);

    assert_eq!(event.event_type(), &EventType::InstructionObserved);
    assert_eq!(
        event.payload()["instruction_activity"]["content_stored"],
        false
    );
    assert_eq!(
        event.payload()["instruction_activity"]["semantic_requirement"],
        false
    );
    assert!(
        event.payload()["instruction_activity"]["content_sha256"]
            .as_str()
            .unwrap()
            .starts_with("sha256-")
    );
    assert!(event.payload().get("requirement_trace").is_none());
}

#[test]
fn write_capable_tool_name_yields_activity_and_an_unverified_gap_only() {
    let (_repo, workspace) = workspace();
    for tool_name in ["Bash", "PowerShell", "shell", "exec"] {
        let mut input = fixture(AdapterHost::Codex, "PostToolUse");
        input["tool_name"] = json!(tool_name);
        let events =
            normalize_hook_input(AdapterHost::Codex, "PostToolUse", input, &workspace).unwrap();
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
}

#[test]
fn supplied_parent_is_preserved_and_absent_parent_is_derived() {
    let (_repo, workspace) = workspace();
    let mut supplied = fixture(AdapterHost::Codex, "SubagentStart");
    supplied["parent_agent_id"] = json!("main-agent");
    let supplied = normalize_hook_input(AdapterHost::Codex, "SubagentStart", supplied, &workspace)
        .unwrap()
        .remove(0);
    let derived = normalize_hook_input(
        AdapterHost::Claude,
        "SubagentStart",
        fixture(AdapterHost::Claude, "SubagentStart"),
        &workspace,
    )
    .unwrap()
    .remove(0);

    assert_eq!(supplied.actor().parent_agent_id(), Some("main-agent"));
    assert_eq!(
        derived.actor().parent_agent_id(),
        Some("claude:phase-1b-session")
    );
}

#[test]
fn unsupported_and_malformed_inputs_fail_closed_without_stopping_the_host() {
    let (repo, workspace) = workspace();
    let mut unsupported = fixture(AdapterHost::Codex, "SessionStart");
    unsupported["hook_event_name"] = json!("FutureLifecycleEvent");
    let output = handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "FutureLifecycleEvent".into(),
        },
        &mut Cursor::new(serde_json::to_vec(&unsupported).unwrap()),
    )
    .unwrap();
    assert_eq!(output.stdout, "{}\n");
    let records = JournalStore::open(&workspace, "phase-1b-session")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(
        records[0].event.payload()["reason"],
        "unsupported_host_event"
    );

    let before = records.len();
    let error = handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "SessionStart".into(),
        },
        &mut Cursor::new(br#"{\"session_id\":"#),
    )
    .unwrap_err();
    assert!(matches!(error, devmap::error::DevMapError::Json(_)));
    assert_eq!(
        JournalStore::open(&workspace, "phase-1b-session")
            .unwrap()
            .replay()
            .unwrap()
            .len(),
        before
    );
}

#[test]
fn successful_hook_projects_presence_after_the_authoritative_journal() {
    let (repo, workspace) = workspace();
    let output = handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "SessionStart".into(),
        },
        &mut Cursor::new(serde_json::to_vec(&fixture(AdapterHost::Codex, "SessionStart")).unwrap()),
    )
    .unwrap();
    assert_eq!(output.exit_code, 0);

    let report = PresenceStore::open(&workspace).unwrap().load_all();
    assert!(report.warnings.is_empty());
    assert_eq!(report.records.len(), 1);
    assert_eq!(report.records[0].status, PresenceStatus::Starting);
}

#[test]
fn presence_failure_never_rolls_back_a_successful_hook_capture() {
    let (repo, workspace) = workspace();
    let devmap_dir = workspace.git_common_dir.join("devmap");
    fs::create_dir_all(&devmap_dir).unwrap();
    fs::write(devmap_dir.join("presence"), b"deliberate conflict").unwrap();

    let output = handle_hook(
        HookHandleArgs {
            source: repo.path().to_path_buf(),
            host: AdapterHost::Codex,
            event: "SessionStart".into(),
        },
        &mut Cursor::new(serde_json::to_vec(&fixture(AdapterHost::Codex, "SessionStart")).unwrap()),
    )
    .unwrap();

    assert_eq!(output.exit_code, 0);
    let records = JournalStore::open(&workspace, "phase-1b-session")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), 1);
    assert_eq!(records[0].event.event_type(), &EventType::SessionStarted);
}

#[test]
fn concurrent_hook_processes_append_a_complete_same_session_journal() {
    let (repo, workspace) = workspace();
    let mut children = Vec::new();
    for source in ["startup", "resume"] {
        let mut child = Command::new(env!("CARGO_BIN_EXE_devmap"))
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
        let mut input = fixture(AdapterHost::Codex, "SessionStart");
        input["source"] = json!(source);
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
    let records = JournalStore::open(&workspace, "phase-1b-session")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].sequence, 1);
    assert_eq!(records[1].sequence, 2);
}
