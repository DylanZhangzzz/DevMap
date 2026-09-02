mod support;

use std::fs;
use std::io::Cursor;

use devmap::capture::{AgentDecisionInput, CaptureKernel, EvidenceInput};
use devmap::cli::{AdapterHost, HookHandleArgs};
use devmap::events::{ActorIdentity, CaptureGrade, HostIdentity, SessionContext};
use devmap::git::{SourceGitInspector, SourceWorkspace};
use devmap::hook::handle_hook;
use devmap::journal::{JournalRecord, JournalStore};
use devmap::mcp::serve_mcp;
use serde_json::{Map, Value, json};
use support::{
    RAW_HOST_PROMPT, SCENARIO_CHILD_AGENT, SCENARIO_MAIN_AGENT, SCENARIO_ROUTE, SCENARIO_SESSION,
    assert_generic_semantic_payloads, assert_native_host_representation,
    assert_native_semantic_payloads, assert_only_source_paths_changed, assert_scenario,
    canonical_semantic_bytes, committed_repo, generic_scenario_expectations,
    native_scenario_expectations, shared_semantic_projection, source_snapshot,
};

const LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";

#[test]
fn official_native_hosts_produce_identical_canonical_semantics() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let source_before = source_snapshot(repository.path());

    let codex = run_native_scenario(repository.path(), &workspace, AdapterHost::Codex);
    assert_scenario(&codex, &native_scenario_expectations());
    assert_native_host_representation(&codex, AdapterHost::Codex);
    assert_native_semantic_payloads(&codex, AdapterHost::Codex, &workspace.head);
    let gaps = codex
        .iter()
        .filter(|record| record.event.event_type() == &devmap::events::EventType::CaptureGap)
        .collect::<Vec<_>>();
    assert_eq!(
        gaps.len(),
        1,
        "an unexplained write must create exactly one gap"
    );
    assert_eq!(gaps[0].event.payload()["reason"], "unexplained_mutation");
    assert_eq!(
        gaps[0].event.payload()["mutation_target"],
        format!("workspace:{}", workspace.head)
    );
    let codex_bytes = codex
        .iter()
        .map(|record| canonical_semantic_bytes(&record.event))
        .collect::<Vec<_>>();
    let codex_journal = fs::read(
        workspace
            .git_dir
            .join("devmap/sessions")
            .join(SCENARIO_SESSION)
            .join("events.ndjson"),
    )
    .unwrap();
    assert!(
        !codex_journal
            .windows(RAW_HOST_PROMPT.len())
            .any(|bytes| bytes == RAW_HOST_PROMPT.as_bytes())
    );

    fs::remove_dir_all(
        workspace
            .git_dir
            .join("devmap/sessions")
            .join(SCENARIO_SESSION),
    )
    .unwrap();
    let claude = run_native_scenario(repository.path(), &workspace, AdapterHost::Claude);
    assert_scenario(&claude, &native_scenario_expectations());
    assert_native_host_representation(&claude, AdapterHost::Claude);
    assert_native_semantic_payloads(&claude, AdapterHost::Claude, &workspace.head);
    let claude_bytes = claude
        .iter()
        .map(|record| canonical_semantic_bytes(&record.event))
        .collect::<Vec<_>>();
    assert_eq!(codex_bytes, claude_bytes);
    let claude_journal = fs::read(
        workspace
            .git_dir
            .join("devmap/sessions")
            .join(SCENARIO_SESSION)
            .join("events.ndjson"),
    )
    .unwrap();
    assert!(
        !claude_journal
            .windows(RAW_HOST_PROMPT.len())
            .any(|bytes| bytes == RAW_HOST_PROMPT.as_bytes())
    );
    assert_only_source_paths_changed(&source_before, &source_snapshot(repository.path()), &[]);
}

#[test]
fn both_generic_mcp_eras_match_codex_and_claude_semantic_projections() {
    let repository = committed_repo();
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let source_before = source_snapshot(repository.path());

    let mut native_projections = Vec::new();
    for host in [AdapterHost::Codex, AdapterHost::Claude] {
        let native = run_native_scenario(repository.path(), &workspace, host);
        assert_scenario(&native, &native_scenario_expectations());
        assert_native_host_representation(&native, host);
        assert_native_semantic_payloads(&native, host, &workspace.head);
        native_projections.push(shared_semantic_projection(&native));
        fs::remove_dir_all(
            workspace
                .git_dir
                .join("devmap/sessions")
                .join(SCENARIO_SESSION),
        )
        .unwrap();
    }
    assert_eq!(native_projections[0], native_projections[1]);

    let mut era_bytes = Vec::new();
    for era in [McpEra::Legacy, McpEra::Modern] {
        let responses = run_generic_scenario(repository.path(), &workspace.head, era);
        assert!(
            responses
                .iter()
                .all(|response| response.get("error").is_none())
        );
        let records = JournalStore::open(&workspace, SCENARIO_SESSION)
            .unwrap()
            .replay()
            .unwrap();
        assert_scenario(&records, &generic_scenario_expectations());
        assert_generic_semantic_payloads(&records, &workspace.head);
        let generic_projection = shared_semantic_projection(&records);
        assert_eq!(generic_projection, native_projections[0]);
        assert_eq!(generic_projection, native_projections[1]);
        era_bytes.push(
            records
                .iter()
                .map(|record| record.event.canonical_bytes().unwrap())
                .collect::<Vec<_>>(),
        );
        fs::remove_dir_all(
            workspace
                .git_dir
                .join("devmap/sessions")
                .join(SCENARIO_SESSION),
        )
        .unwrap();
    }
    assert_eq!(era_bytes[0], era_bytes[1]);
    assert_only_source_paths_changed(&source_before, &source_snapshot(repository.path()), &[]);
}

fn run_native_scenario(
    source: &std::path::Path,
    workspace: &SourceWorkspace,
    host: AdapterHost,
) -> Vec<JournalRecord> {
    native_hook(
        source,
        host,
        "SessionStart",
        native_input(host, "evt-01", SCENARIO_MAIN_AGENT, None, None),
    );
    native_hook(
        source,
        host,
        "UserPromptSubmit",
        native_input(
            host,
            "evt-02",
            SCENARIO_MAIN_AGENT,
            None,
            Some(RAW_HOST_PROMPT),
        ),
    );
    native_hook(
        source,
        host,
        "SubagentStart",
        native_input(
            host,
            "evt-03",
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            None,
        ),
    );

    semantic_kernel(workspace, host)
        .record_decision(
            "evt-04",
            "2026-08-27T12:00:03Z",
            AgentDecisionInput {
                decision: "Use a compatibility adapter".into(),
                basis: vec!["Both supported hosts expose lifecycle hooks".into()],
                alternatives: vec!["Duplicate host-specific capture logic".into()],
                rationale: "A shared kernel preserves equivalent semantics".into(),
                scope: "Phase 1B native capture".into(),
                authority: "approved Phase 1B plan".into(),
                revisit_trigger: "A host cannot express the canonical contract".into(),
            },
        )
        .unwrap();

    let mut write = native_input(
        host,
        "evt-05",
        SCENARIO_CHILD_AGENT,
        Some(SCENARIO_MAIN_AGENT),
        None,
    );
    let object = write.as_object_mut().unwrap();
    match host {
        AdapterHost::Codex => {
            object.insert("tool_name".into(), json!("apply_patch"));
            object.insert("tool_input".into(), json!({"path": "src/lib.rs"}));
            object.insert("tool_response".into(), json!({"status": "ok"}));
        }
        AdapterHost::Claude => {
            object.insert("toolName".into(), json!("apply_patch"));
            object.insert("tool_input".into(), json!({"path": "src/lib.rs"}));
            object.insert("tool_response".into(), json!({"status": "ok"}));
        }
        AdapterHost::GenericMcp => unreachable!(),
    }
    native_hook(source, host, "PostToolUse", write);

    semantic_kernel(workspace, host)
        .record_evidence(
            "evt-08",
            "2026-08-27T12:00:05Z",
            EvidenceInput {
                kind: "test".into(),
                target: format!("commit:{}", workspace.head),
                command: Some("cargo test --all-targets --all-features".into()),
                outcome: "passed".into(),
            },
        )
        .unwrap();

    native_hook(
        source,
        host,
        "PreCompact",
        native_input(
            host,
            "evt-09",
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            None,
        ),
    );
    native_hook(
        source,
        host,
        "PostCompact",
        native_input(
            host,
            "evt-10",
            SCENARIO_CHILD_AGENT,
            Some(SCENARIO_MAIN_AGENT),
            None,
        ),
    );
    native_hook(
        source,
        host,
        "Stop",
        native_input(host, "evt-11", SCENARIO_MAIN_AGENT, None, None),
    );

    JournalStore::open(workspace, SCENARIO_SESSION)
        .unwrap()
        .replay()
        .unwrap()
}

fn semantic_kernel(workspace: &SourceWorkspace, host: AdapterHost) -> CaptureKernel {
    let host_name = match host {
        AdapterHost::Codex => "codex",
        AdapterHost::Claude => "claude",
        AdapterHost::GenericMcp => unreachable!(),
    };
    CaptureKernel::new(
        JournalStore::open(workspace, SCENARIO_SESSION).unwrap(),
        CaptureGrade::A,
        HostIdentity::new(host_name, "devmap-hook/1").unwrap(),
        ActorIdentity::new(SCENARIO_CHILD_AGENT, Some(SCENARIO_MAIN_AGENT.into())).unwrap(),
        SessionContext::new(
            SCENARIO_SESSION,
            Some(SCENARIO_ROUTE.into()),
            workspace.root.to_string_lossy(),
            Some(workspace.root.to_string_lossy().into_owned()),
            workspace.branch.clone(),
            Some(workspace.head.clone()),
        )
        .unwrap(),
    )
}

fn native_hook(source: &std::path::Path, host: AdapterHost, event: &str, input: Value) {
    let mut stdin = Cursor::new(serde_json::to_vec(&input).unwrap());
    let output = handle_hook(
        HookHandleArgs {
            source: source.to_path_buf(),
            host,
            event: event.into(),
        },
        &mut stdin,
    )
    .unwrap();
    assert_eq!(output.exit_code, 0);
    assert_eq!(output.stdout, "{}\n");
}

fn native_input(
    host: AdapterHost,
    event_id: &str,
    actor: &str,
    parent: Option<&str>,
    prompt: Option<&str>,
) -> Value {
    let event_number: u8 = event_id.trim_start_matches("evt-").parse().unwrap();
    let mut input = Map::new();
    match host {
        AdapterHost::Codex => {
            input.insert("thread_id".into(), json!(SCENARIO_SESSION));
            input.insert("hook_event_id".into(), json!(event_id));
            input.insert("agent_id".into(), json!(actor));
            input.insert("route_id".into(), json!(SCENARIO_ROUTE));
            input.insert(
                "timestamp".into(),
                json!(format!("2026-08-27T12:00:{event_number:02}Z")),
            );
            input.insert("cwd".into(), json!("/workspace/devmap"));
        }
        AdapterHost::Claude => {
            input.insert("sessionId".into(), json!(SCENARIO_SESSION));
            input.insert("event_id".into(), json!(event_id));
            input.insert("agentId".into(), json!(actor));
            input.insert("routeId".into(), json!(SCENARIO_ROUTE));
            input.insert(
                "occurred_at".into(),
                json!(format!("2026-08-27T12:00:{event_number:02}Z")),
            );
            input.insert("transcript_path".into(), json!("/host/session.jsonl"));
        }
        AdapterHost::GenericMcp => unreachable!(),
    }
    if let Some(parent) = parent {
        let key = if host == AdapterHost::Claude {
            "parentAgentId"
        } else {
            "parent_agent_id"
        };
        input.insert(key.into(), json!(parent));
    }
    if let Some(prompt) = prompt {
        input.insert("prompt".into(), json!(prompt));
    }
    Value::Object(input)
}

#[derive(Clone, Copy)]
enum McpEra {
    Legacy,
    Modern,
}

fn run_generic_scenario(source: &std::path::Path, head: &str, era: McpEra) -> Vec<Value> {
    let calls = vec![
        mcp_call(
            2,
            "devmap_record_requirement",
            json!({
                "session_id": SCENARIO_SESSION,
                "agent_id": SCENARIO_MAIN_AGENT,
                "route_id": SCENARIO_ROUTE,
                "event_id": "evt-02",
                "occurred_at": "2026-08-27T12:00:02Z",
                "source_kind": "human_instruction",
                "source_locator": "turn:1",
                "quoted_text": "Approved requirement quotation"
            }),
            era,
        ),
        mcp_call(
            3,
            "devmap_record_decision",
            json!({
                "session_id": SCENARIO_SESSION,
                "agent_id": SCENARIO_CHILD_AGENT,
                "parent_agent_id": SCENARIO_MAIN_AGENT,
                "route_id": SCENARIO_ROUTE,
                "event_id": "evt-04",
                "occurred_at": "2026-08-27T12:00:03Z",
                "decision": "Use a compatibility adapter",
                "basis": ["Both supported hosts expose lifecycle hooks"],
                "alternatives": ["Duplicate host-specific capture logic"],
                "rationale": "A shared kernel preserves equivalent semantics",
                "scope": "Phase 1B native capture",
                "authority": "approved Phase 1B plan",
                "revisit_trigger": "A host cannot express the canonical contract"
            }),
            era,
        ),
        mcp_call(
            4,
            "devmap_record_evidence",
            json!({
                "session_id": SCENARIO_SESSION,
                "agent_id": SCENARIO_CHILD_AGENT,
                "parent_agent_id": SCENARIO_MAIN_AGENT,
                "route_id": SCENARIO_ROUTE,
                "event_id": "evt-08",
                "occurred_at": "2026-08-27T12:00:05Z",
                "kind": "test",
                "target": format!("commit:{head}"),
                "command": "cargo test --all-targets --all-features",
                "outcome": "passed"
            }),
            era,
        ),
    ];
    let mut messages = Vec::new();
    match era {
        McpEra::Legacy => messages.push(json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": LEGACY_PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": {"name": "conformance", "version": "1.0.0"}
            }
        })),
        McpEra::Modern => messages.push(modern_request(1, "server/discover", json!({}))),
    }
    messages.extend(calls);
    let input = messages
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let mut output = Vec::new();
    serve_mcp(source, Cursor::new(input.into_bytes()), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn mcp_call(id: u8, name: &str, arguments: Value, era: McpEra) -> Value {
    let params = json!({"name": name, "arguments": arguments});
    match era {
        McpEra::Legacy => {
            json!({"jsonrpc": "2.0", "id": id, "method": "tools/call", "params": params})
        }
        McpEra::Modern => modern_request(id, "tools/call", params),
    }
}

fn modern_request(id: u8, method: &str, mut params: Value) -> Value {
    params.as_object_mut().unwrap().insert(
        "_meta".into(),
        json!({
            "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientInfo": {"name": "conformance", "version": "1.0.0"},
            "io.modelcontextprotocol/clientCapabilities": {}
        }),
    );
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}
