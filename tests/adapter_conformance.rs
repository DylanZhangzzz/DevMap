mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use devmap::adapter::{HookBinding, install_adapter, plan_adapter};
use devmap::cli::AdapterHost;
use devmap::events::{CaptureGrade, EventType};
use devmap::git::SourceGitInspector;
use devmap::journal::{JournalRecord, JournalStore};
use serde_json::{Value, json};
use support::{
    SCENARIO_CHILD_AGENT, SCENARIO_MAIN_AGENT, SCENARIO_ROUTE, SCENARIO_SESSION,
    assert_only_source_paths_changed, committed_repo, source_snapshot,
};

const LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";
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
const EVIDENCE_TARGET: &str =
    "artifact:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
const PRIVACY_CANARIES: [&str; 10] = [
    "CANARY_CODEX_TRANSCRIPT_SECRET",
    "CANARY_CODEX_CWD_SECRET",
    "CANARY_CLAUDE_TRANSCRIPT_SECRET",
    "CANARY_CLAUDE_CWD_SECRET",
    "CANARY_PROMPT_SECRET",
    "CANARY_COMMAND_SECRET",
    "CANARY_OUTPUT_SECRET",
    "CANARY_COMPACT_SECRET",
    "CANARY_ASSISTANT_SECRET",
    "CANARY_SUBAGENT_TRANSCRIPT_SECRET",
];

#[test]
fn installed_native_bindings_and_real_mcp_share_a_truthful_canonical_contract() {
    let codex = run_host_scenario(AdapterHost::Codex);
    let claude = run_host_scenario(AdapterHost::Claude);

    // The comparison deliberately keeps route, actor/parent, sequence, derived grade, and
    // evidence. Only vendor identity is represented by a documented semantic role.
    assert_eq!(
        canonical_contract(&codex, AdapterHost::Codex),
        canonical_contract(&claude, AdapterHost::Claude)
    );
}

#[test]
fn installed_binding_turns_an_event_name_mismatch_into_an_honest_gap() {
    for host in [AdapterHost::Codex, AdapterHost::Claude] {
        let repository = committed_repo();
        let plan = plan_adapter(repository.path(), host).unwrap();
        let binding = plan
            .bindings
            .iter()
            .find(|binding| binding.event == "SessionStart")
            .unwrap()
            .clone();
        let token = plan.plan_digest.clone();
        install_adapter(plan, &token).unwrap();

        let mut input = fixture(host, "SessionStart");
        input["session_id"] = json!(format!("mismatch-{}", host_name(host)));
        input["hook_event_name"] = json!("Stop");
        run_installed_binding(repository.path(), &binding, &input);

        let workspace = SourceGitInspector::open(repository.path())
            .unwrap()
            .workspace()
            .unwrap();
        let records = JournalStore::open(&workspace, &format!("mismatch-{}", host_name(host)))
            .unwrap()
            .replay()
            .unwrap();
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].event.event_type(), &EventType::CaptureGap);
        assert_eq!(records[0].event.payload()["reason"], "host_event_mismatch");
        assert_eq!(records[0].event.payload()["capture_grade"], "D");
    }
}

fn run_host_scenario(host: AdapterHost) -> Vec<JournalRecord> {
    let repository = committed_repo();
    let before = source_snapshot(repository.path());
    let plan = plan_adapter(repository.path(), host).unwrap();
    let config_path = plan.config_path.clone();
    let relative_config = config_path
        .strip_prefix(repository.path())
        .unwrap()
        .to_owned();
    let bindings = plan
        .bindings
        .iter()
        .map(|binding| (binding.event.clone(), binding.clone()))
        .collect::<BTreeMap<_, _>>();
    let token = plan.plan_digest.clone();
    install_adapter(plan, &token).unwrap();

    for event in EVENTS {
        let input = fixture(host, event);
        let binding = bindings.get(event).unwrap();
        run_installed_binding(repository.path(), binding, &input);
        if event == "PostToolUse" {
            run_installed_binding(repository.path(), binding, &input);
        }
    }

    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let modern = run_mcp_process(repository.path(), McpEra::Modern);
    assert_successful_responses(&modern);
    let legacy_retry = run_mcp_process(repository.path(), McpEra::Legacy);
    assert_successful_responses(&legacy_retry);

    let records = JournalStore::open(&workspace, SCENARIO_SESSION)
        .unwrap()
        .replay()
        .unwrap();
    assert_truthful_scenario(&records, host);
    let journal = fs::read(
        workspace
            .git_dir
            .join("devmap/sessions")
            .join(SCENARIO_SESSION)
            .join("events.ndjson"),
    )
    .unwrap();
    let persisted = String::from_utf8(journal).unwrap();
    for canary in PRIVACY_CANARIES {
        assert!(
            !persisted.contains(canary),
            "persisted privacy canary {canary}"
        );
    }
    assert_only_source_paths_changed(
        &before,
        &source_snapshot(repository.path()),
        &[relative_config.as_path()],
    );
    records
}

fn assert_truthful_scenario(records: &[JournalRecord], host: AdapterHost) {
    use EventType::*;
    let expected_types = [
        SessionStarted,
        InstructionObserved,
        ToolRequested,
        ToolCompleted,
        CaptureGap,
        ContextCompacting,
        ContextCompacted,
        AgentStarted,
        AgentStopped,
        TurnCompleted,
        SessionStopped,
        InstructionObserved,
        DecisionRecorded,
        EvidenceRecorded,
    ];
    assert_eq!(
        records.len(),
        expected_types.len(),
        "retry duplicated a hook or MCP call"
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record.event.event_type().clone())
            .collect::<Vec<_>>(),
        expected_types
    );
    assert!(
        !records
            .iter()
            .any(|record| record.event.event_type() == &MutationObserved)
    );

    for (index, record) in records.iter().enumerate() {
        assert_eq!(record.sequence, index as u64 + 1);
        assert_eq!(record.event.sequence(), record.sequence);
        assert_eq!(record.event.payload()["capture_grade"], "D");
        if index < 11 {
            assert_eq!(record.event.host().name(), host_name(host));
            assert_eq!(record.event.context().route_id(), None);
        } else {
            assert_eq!(record.event.host().name(), "generic_mcp");
            assert_eq!(record.event.context().route_id(), Some(SCENARIO_ROUTE));
        }
    }

    let session_actor = format!("{}:{SCENARIO_SESSION}", host_name(host));
    for index in [0, 1, 2, 3, 4, 5, 6, 9, 10] {
        assert_eq!(records[index].event.actor().agent_id(), session_actor);
        assert_eq!(records[index].event.actor().parent_agent_id(), None);
    }
    for index in [7, 8] {
        assert_eq!(
            records[index].event.actor().agent_id(),
            SCENARIO_CHILD_AGENT
        );
        assert_eq!(
            records[index].event.actor().parent_agent_id(),
            Some(session_actor.as_str())
        );
    }
    assert_eq!(records[4].event.payload()["reason"], "mutation_unverified");
    assert!(records[4].event.payload().get("mutation_target").is_none());
    assert_eq!(records[9].event.payload()["activity"], "turn_completed");
    assert_eq!(records[10].event.payload()["activity"], "session_stopped");
    assert_eq!(
        records[1].event.payload()["instruction_activity"]["semantic_requirement"],
        false
    );
    assert!(records[..11].iter().all(|record| {
        record.event.payload().get("requirement_trace").is_none()
            && record.event.payload().get("agent_decision").is_none()
            && record.event.payload().get("evidence").is_none()
    }));

    assert_eq!(records[11].event.actor().agent_id(), SCENARIO_MAIN_AGENT);
    assert_eq!(records[11].event.actor().parent_agent_id(), None);
    assert_eq!(
        records[11].event.payload()["requirement_trace"]["approved_quotation"],
        "Approved requirement quotation"
    );
    for index in [12, 13] {
        assert_eq!(
            records[index].event.actor().agent_id(),
            SCENARIO_CHILD_AGENT
        );
        assert_eq!(
            records[index].event.actor().parent_agent_id(),
            Some(SCENARIO_MAIN_AGENT)
        );
    }
    assert_eq!(
        records[13].event.payload()["evidence"],
        json!({
            "kind": "test",
            "target": EVIDENCE_TARGET,
            "command": "cargo test --all-targets --all-features",
            "outcome": "passed"
        })
    );
    assert_eq!(records[13].event.payload()["provisional"], false);
}

#[derive(Debug, PartialEq)]
struct ContractEvent {
    event_type: EventType,
    sequence: u64,
    capture_grade: CaptureGrade,
    route_id: Option<String>,
    host_role: &'static str,
    actor_role: String,
    parent_role: Option<String>,
    semantic_evidence: Value,
}

fn canonical_contract(records: &[JournalRecord], host: AdapterHost) -> Vec<ContractEvent> {
    let native_actor = format!("{}:{SCENARIO_SESSION}", host_name(host));
    records
        .iter()
        .map(|record| {
            let actor_role = normalize_actor(record.event.actor().agent_id(), &native_actor);
            let parent_role = record
                .event
                .actor()
                .parent_agent_id()
                .map(|parent| normalize_actor(parent, &native_actor));
            ContractEvent {
                event_type: record.event.event_type().clone(),
                sequence: record.sequence,
                capture_grade: serde_json::from_value(
                    record.event.payload()["capture_grade"].clone(),
                )
                .unwrap(),
                route_id: record.event.context().route_id().map(str::to_owned),
                host_role: if record.event.host().name() == "generic_mcp" {
                    "semantic_mcp"
                } else {
                    "native_activity"
                },
                actor_role,
                parent_role,
                semantic_evidence: record
                    .event
                    .payload()
                    .get("evidence")
                    .cloned()
                    .unwrap_or(Value::Null),
            }
        })
        .collect()
}

fn normalize_actor(actor: &str, native_actor: &str) -> String {
    if actor == native_actor {
        "native-session-actor".to_owned()
    } else {
        actor.to_owned()
    }
}

fn fixture(host: AdapterHost, event: &str) -> Value {
    let raw = match host {
        AdapterHost::Codex => include_str!("fixtures/hooks/codex-events.json"),
        AdapterHost::Claude => include_str!("fixtures/hooks/claude-events.json"),
        AdapterHost::GenericMcp => unreachable!(),
    };
    serde_json::from_str::<BTreeMap<String, Value>>(raw).unwrap()[event].clone()
}

fn run_installed_binding(source: &Path, binding: &HookBinding, input: &Value) {
    let mut words = binding.command.split_whitespace();
    assert_eq!(words.next(), Some("devmap"));
    let mut child = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args(words)
        .current_dir(source)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&serde_json::to_vec(input).unwrap())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "installed binding failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(String::from_utf8(output.stdout).unwrap(), "{}\n");
}

#[derive(Clone, Copy)]
enum McpEra {
    Legacy,
    Modern,
}

fn run_mcp_process(source: &Path, era: McpEra) -> Vec<Value> {
    let calls = [
        mcp_call(
            2,
            "devmap_record_requirement",
            json!({
                "session_id": SCENARIO_SESSION,
                "agent_id": SCENARIO_MAIN_AGENT,
                "route_id": SCENARIO_ROUTE,
                "event_id": "semantic-requirement",
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
                "event_id": "semantic-decision",
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
                "event_id": "semantic-evidence",
                "occurred_at": "2026-08-27T12:00:05Z",
                "kind": "test",
                "target": EVIDENCE_TARGET,
                "command": "cargo test --all-targets --all-features",
                "outcome": "passed"
            }),
            era,
        ),
    ];
    let mut messages = Vec::with_capacity(4);
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
    let mut child = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args(["mcp", "--source", source.to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(input.as_bytes())
        .unwrap();
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "MCP process failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

fn assert_successful_responses(responses: &[Value]) {
    assert_eq!(responses.len(), 4);
    assert!(
        responses
            .iter()
            .all(|response| response.get("result").is_some())
    );
    assert!(
        responses
            .iter()
            .all(|response| response.get("error").is_none())
    );
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

fn host_name(host: AdapterHost) -> &'static str {
    match host {
        AdapterHost::Codex => "codex",
        AdapterHost::Claude => "claude",
        AdapterHost::GenericMcp => "generic_mcp",
    }
}
