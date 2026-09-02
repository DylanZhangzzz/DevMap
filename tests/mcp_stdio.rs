mod support;

use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};

use devmap::git::SourceGitInspector;
use devmap::journal::JournalStore;
use devmap::mcp::{MCP_TOOLS, serve_mcp};
use serde_json::{Value, json};

use support::{committed_repo, git};

const PROTOCOL_VERSION: &str = "2025-11-25";

fn request(id: Value, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn call(id: Value, name: &str, arguments: Value) -> Value {
    request(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
}

fn run_stream(source: &Path, messages: &[Value]) -> Vec<Value> {
    let mut input = messages
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    input.push('\n');
    let mut output = Vec::new();

    serve_mcp(source, Cursor::new(input.into_bytes()), &mut output).unwrap();

    let text = String::from_utf8(output).expect("MCP output must be UTF-8");
    assert!(text.ends_with('\n'));
    text.lines()
        .map(|line| {
            assert!(!line.is_empty());
            serde_json::from_str(line).expect("every stdout line must be one JSON-RPC message")
        })
        .collect()
}

fn initialize(id: Value) -> Value {
    request(
        id,
        "initialize",
        json!({
            "protocolVersion": PROTOCOL_VERSION,
            "capabilities": {},
            "clientInfo": {"name": "devmap-test", "version": "1.0.0"}
        }),
    )
}

#[test]
fn stdio_handles_initialize_list_all_tools_and_multiple_messages() {
    let repository = committed_repo();
    let before = git_state(repository.path());
    let responses = run_stream(
        repository.path(),
        &[
            initialize(json!(1)),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            request(json!("tools"), "tools/list", json!({})),
            call(json!(2), "devmap_context", json!({})),
            call(
                json!(3),
                "devmap_record_requirement",
                json!({
                    "session_id": "mcp-session",
                    "agent_id": "agent-1",
                    "event_id": "evt-requirement",
                    "occurred_at": "2026-09-02T12:00:00Z",
                    "source_kind": "human_instruction",
                    "source_locator": "turn:7",
                    "quoted_text": "Keep only this approved quotation."
                }),
            ),
            call(
                json!(4),
                "devmap_record_decision",
                json!({
                    "session_id": "mcp-session",
                    "agent_id": "agent-1",
                    "event_id": "evt-decision",
                    "occurred_at": "2026-09-02T12:00:01Z",
                    "decision": "Use the stdio transport.",
                    "basis": ["The adapter is local."],
                    "alternatives": ["HTTP"],
                    "rationale": "It is the requested minimal transport.",
                    "scope": "Phase 1B generic adapter",
                    "authority": "approved implementation plan",
                    "revisit_trigger": "A later phase requires remote transport."
                }),
            ),
            call(
                json!(5),
                "devmap_record_evidence",
                json!({
                    "session_id": "mcp-session",
                    "agent_id": "agent-1",
                    "event_id": "evt-evidence",
                    "occurred_at": "2026-09-02T12:00:02Z",
                    "kind": "test",
                    "target": format!("commit:{}", git(repository.path(), ["rev-parse", "HEAD"])),
                    "command": "cargo test --test mcp_stdio",
                    "outcome": "passed"
                }),
            ),
        ],
    );

    assert_eq!(
        responses.len(),
        6,
        "the notification must not receive a response"
    );
    assert_eq!(responses[0]["id"], 1);
    assert_eq!(responses[0]["result"]["protocolVersion"], PROTOCOL_VERSION);
    assert_eq!(responses[0]["result"]["capabilities"], json!({"tools": {}}));
    assert_eq!(responses[0]["result"]["serverInfo"]["name"], "devmap");

    assert_eq!(responses[1]["id"], "tools");
    let listed = responses[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(listed, MCP_TOOLS);
    assert!(
        responses[1]["result"]["tools"]
            .as_array()
            .unwrap()
            .iter()
            .all(|tool| tool["inputSchema"]["type"] == "object")
    );

    let context = &responses[2]["result"]["structuredContent"];
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    assert_eq!(
        context["workspace"],
        workspace.root.to_string_lossy().as_ref()
    );
    assert_eq!(context["branch"], "main");
    assert_eq!(context["head"], workspace.head);
    assert_eq!(context["capture_grade"], "C");
    assert_eq!(
        context["journal_location"],
        workspace
            .git_dir
            .join("devmap/sessions")
            .to_string_lossy()
            .as_ref()
    );

    let records = JournalStore::open(&workspace, "mcp-session")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), 3);
    for (response, record) in responses[3..].iter().zip(&records) {
        assert_eq!(
            response["result"]["structuredContent"]["sha256"],
            record.sha256
        );
    }
    assert_eq!(
        records[0].event.payload()["requirement_trace"]["approved_quotation"],
        "Keep only this approved quotation."
    );
    assert!(records[0].event.payload().get("agent_decision").is_none());
    assert_eq!(git_state(repository.path()), before);
}

#[test]
fn stdio_preserves_ids_and_returns_json_rpc_errors_without_answering_notifications() {
    let repository = committed_repo();
    let responses = run_stream(
        repository.path(),
        &[
            request(
                json!("unsupported"),
                "initialize",
                json!({
                    "protocolVersion": "1900-01-01",
                    "capabilities": {},
                    "clientInfo": {"name": "old-client", "version": "1"}
                }),
            ),
            initialize(json!(7)),
            request(json!(1.5), "tools/list", json!({})),
            request(json!("bad-params"), "tools/call", json!({"arguments": {}})),
            request(json!(8), "unknown/method", json!({})),
            json!({"jsonrpc": "2.0", "method": "unknown/notification", "params": {}}),
        ],
    );

    assert_eq!(responses.len(), 5);
    assert_eq!(responses[0]["id"], "unsupported");
    assert_eq!(responses[0]["error"]["code"], -32602);
    assert_eq!(
        responses[0]["error"]["data"]["supported"],
        json!([PROTOCOL_VERSION])
    );
    assert_eq!(responses[0]["error"]["data"]["requested"], "1900-01-01");
    assert_eq!(responses[1]["id"], 7);
    assert_eq!(responses[2]["id"], 1.5);
    assert!(responses[2].get("result").is_some());
    assert_eq!(responses[3]["id"], "bad-params");
    assert_eq!(responses[3]["error"]["code"], -32602);
    assert_eq!(responses[4]["id"], 8);
    assert_eq!(responses[4]["error"]["code"], -32601);
}

#[test]
fn malformed_request_without_method_is_not_mistaken_for_a_notification() {
    let repository = committed_repo();
    let responses = run_stream(
        repository.path(),
        &[
            json!({"jsonrpc": "2.0", "params": {}}),
            json!({"jsonrpc": "2.0", "id": null, "method": "tools/list"}),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
        ],
    );

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["id"], Value::Null);
    assert_eq!(responses[0]["error"]["code"], -32600);
    assert_eq!(responses[1]["id"], Value::Null);
    assert_eq!(responses[1]["error"]["code"], -32600);
}

#[test]
fn tool_validation_errors_are_visible_and_do_not_append_records() {
    let repository = committed_repo();
    let responses = run_stream(
        repository.path(),
        &[
            initialize(json!(1)),
            call(
                json!(2),
                "devmap_record_decision",
                json!({
                    "session_id": "invalid-session",
                    "agent_id": "agent-1",
                    "event_id": "invalid-decision",
                    "occurred_at": "2026-09-02T12:00:00Z",
                    "decision": "Skip validation.",
                    "basis": ["Faster"],
                    "alternatives": [],
                    "rationale": "Convenient",
                    "scope": "material route",
                    "authority": "maintainer",
                    "revisit_trigger": "Never"
                }),
            ),
            call(
                json!(3),
                "devmap_record_evidence",
                json!({
                    "session_id": "invalid-session",
                    "agent_id": "agent-1",
                    "event_id": "invalid-evidence",
                    "occurred_at": "2026-09-02T12:00:01Z",
                    "kind": "test",
                    "target": "branch:main",
                    "outcome": "passed"
                }),
            ),
            call(
                json!(4),
                "devmap_record_requirement",
                json!({
                    "session_id": "invalid-session",
                    "agent_id": "agent-1",
                    "event_id": "raw-requirement",
                    "occurred_at": "2026-09-02T12:00:02Z",
                    "source_kind": "human_instruction",
                    "quoted_text": "Approved excerpt",
                    "raw_transcript_opt_in": true
                }),
            ),
        ],
    );

    for response in &responses[1..] {
        assert_eq!(response["result"]["isError"], true);
        assert_eq!(response["result"]["content"][0]["type"], "text");
    }
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    assert!(
        JournalStore::open(&workspace, "invalid-session")
            .unwrap()
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn identical_explicit_capture_input_produces_the_same_record_hash() {
    let repository = committed_repo();
    let message = call(
        json!(2),
        "devmap_record_requirement",
        json!({
            "session_id": "deterministic-session",
            "agent_id": "agent-1",
            "event_id": "deterministic-event",
            "occurred_at": "2026-09-02T12:00:00Z",
            "source_kind": "human_instruction",
            "source_locator": "turn:10",
            "quoted_text": "Use deterministic canonical records."
        }),
    );
    let first = run_stream(repository.path(), &[initialize(json!(1)), message.clone()]);
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    fs::remove_dir_all(
        workspace
            .git_dir
            .join("devmap/sessions/deterministic-session"),
    )
    .unwrap();
    let second = run_stream(repository.path(), &[initialize(json!(1)), message]);

    assert_eq!(
        first[1]["result"]["structuredContent"]["sha256"],
        second[1]["result"]["structuredContent"]["sha256"]
    );
}

#[test]
fn executable_writes_no_diagnostics_or_banners_to_stdout() {
    let repository = committed_repo();
    let mut child = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args(["mcp", "--source", repository.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    {
        use std::io::Write;
        let stdin = child.stdin.as_mut().unwrap();
        writeln!(stdin, "{}", initialize(json!(1))).unwrap();
        writeln!(stdin, "{{not JSON").unwrap();
    }
    let output = child.wait_with_output().unwrap();

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).unwrap();
    let messages = stdout
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    assert_eq!(messages.len(), 2);
    assert_eq!(messages[1]["error"]["code"], -32700);
    assert_eq!(messages[1]["id"], Value::Null);
}

fn git_state(root: &Path) -> (String, String, String, String, String) {
    (
        git(root, ["rev-parse", "HEAD"]),
        git(root, ["branch", "--show-current"]),
        git(root, ["ls-files", "--stage"]),
        git(root, ["for-each-ref", "--format=%(refname):%(objectname)"]),
        git(root, ["config", "--local", "--list", "--show-origin"]),
    )
}
