mod support;

use std::fs;
use std::io::Cursor;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Barrier};
use std::thread;

use devmap::git::SourceGitInspector;
use devmap::journal::JournalStore;
use devmap::mcp::{MCP_TOOLS, serve_mcp};
use devmap::presence::{PresenceStatus, PresenceStore};
use serde_json::{Value, json};

use support::{committed_repo, git};

const LEGACY_PROTOCOL_VERSION: &str = "2025-11-25";
const MODERN_PROTOCOL_VERSION: &str = "2026-07-28";

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

fn modern_request(id: Value, method: &str, params: Value) -> Value {
    let mut params = params.as_object().unwrap().clone();
    params.insert(
        "_meta".into(),
        json!({
            "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
            "io.modelcontextprotocol/clientInfo": {"name": "devmap-modern-test", "version": "1.0.0"},
            "io.modelcontextprotocol/clientCapabilities": {}
        }),
    );
    request(id, method, Value::Object(params))
}

fn modern_call(id: Value, name: &str, arguments: Value) -> Value {
    modern_request(
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
            "protocolVersion": LEGACY_PROTOCOL_VERSION,
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
    assert_eq!(
        responses[0]["result"]["protocolVersion"],
        LEGACY_PROTOCOL_VERSION
    );
    assert_eq!(
        responses[0]["result"]["capabilities"],
        json!({"resources": {}, "tools": {}})
    );
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

    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let presence = PresenceStore::open(&workspace).unwrap().load_all();
    assert_eq!(presence.records.len(), 1);
    assert_eq!(presence.records[0].status, PresenceStatus::Working);

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
    assert_eq!(context["capture_grade"], "D");
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
    assert_eq!(
        responses[0]["result"]["protocolVersion"],
        LEGACY_PROTOCOL_VERSION
    );
    assert_eq!(
        responses[0]["result"]["capabilities"],
        json!({"resources": {}, "tools": {}})
    );
    assert!(responses[0].get("error").is_none());
    assert_eq!(responses[1]["id"], 7);
    assert_eq!(responses[2]["id"], 1.5);
    assert!(responses[2].get("result").is_some());
    assert_eq!(responses[3]["id"], "bad-params");
    assert_eq!(responses[3]["error"]["code"], -32602);
    assert_eq!(responses[4]["id"], 8);
    assert_eq!(responses[4]["error"]["code"], -32601);
}

#[test]
fn dual_era_stdio_serves_modern_discovery_and_stateless_tools_alongside_legacy() {
    let repository = committed_repo();
    let responses = run_stream(
        repository.path(),
        &[
            modern_request(json!("discover"), "server/discover", json!({})),
            modern_request(json!("modern-list"), "tools/list", json!({})),
            modern_call(json!("modern-call"), "devmap_context", json!({})),
            json!({
                "jsonrpc": "2.0",
                "method": "notifications/cancelled",
                "params": {
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION,
                        "io.modelcontextprotocol/clientCapabilities": {}
                    },
                    "requestId": "not-running"
                }
            }),
            initialize(json!("legacy-init")),
            request(json!("legacy-list"), "tools/list", json!({})),
        ],
    );

    assert_eq!(
        responses.len(),
        5,
        "modern notifications remain response-free"
    );
    let discovery = &responses[0]["result"];
    assert_eq!(discovery["resultType"], "complete");
    assert_eq!(
        discovery["supportedVersions"],
        json!([MODERN_PROTOCOL_VERSION])
    );
    assert_eq!(
        discovery["capabilities"],
        json!({"resources": {}, "tools": {}})
    );
    assert_eq!(
        discovery["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "devmap"
    );

    for response in &responses[1..=2] {
        assert_eq!(response["result"]["resultType"], "complete");
        assert_eq!(
            response["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["version"],
            env!("CARGO_PKG_VERSION")
        );
    }
    assert_eq!(responses[1]["id"], "modern-list");
    assert_eq!(responses[2]["id"], "modern-call");
    assert_eq!(
        responses[3]["result"]["protocolVersion"],
        LEGACY_PROTOCOL_VERSION
    );
    assert!(responses[3]["result"].get("resultType").is_none());
    assert_eq!(responses[4]["id"], "legacy-list");
    assert!(responses[4]["result"].get("resultType").is_none());
}

#[test]
fn modern_discovery_and_tools_list_emit_required_conservative_cache_fields() {
    let repository = committed_repo();
    let responses = run_stream(
        repository.path(),
        &[
            modern_request(json!("discover"), "server/discover", json!({})),
            modern_request(json!("list"), "tools/list", json!({})),
        ],
    );

    assert_eq!(responses.len(), 2);
    for response in responses {
        assert_eq!(response["result"]["ttlMs"], 0);
        assert_eq!(response["result"]["cacheScope"], "private");
        assert_eq!(
            response["result"]["_meta"],
            json!({
                "io.modelcontextprotocol/serverInfo": {
                    "name": "devmap",
                    "version": env!("CARGO_PKG_VERSION")
                }
            })
        );
        assert!(response.get("ttlMs").is_none());
        assert!(response["result"]["_meta"].get("ttlMs").is_none());
        assert!(response["result"]["_meta"].get("cacheScope").is_none());
    }
}

#[test]
fn legacy_metadata_without_namespaced_protocol_version_stays_legacy_after_initialize() {
    let repository = committed_repo();
    let responses = run_stream(
        repository.path(),
        &[
            initialize(json!("initialize")),
            json!({"jsonrpc": "2.0", "method": "notifications/initialized"}),
            request(
                json!("legacy-list"),
                "tools/list",
                json!({
                    "_meta": {
                        "progressToken": "legacy-progress",
                        "com.example/requestTag": "legacy-metadata"
                    }
                }),
            ),
        ],
    );

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[1]["id"], "legacy-list");
    assert!(responses[1].get("error").is_none());
    assert!(responses[1]["result"]["tools"].is_array());
    for modern_field in ["resultType", "ttlMs", "cacheScope"] {
        assert!(responses[1]["result"].get(modern_field).is_none());
    }
}

#[test]
fn modern_rejects_bad_metadata_while_legacy_initialize_counteroffers() {
    let repository = committed_repo();
    let unsupported_modern = |id: &str, version: &str| {
        request(
            json!(id),
            "tools/list",
            json!({
                "_meta": {
                    "io.modelcontextprotocol/protocolVersion": version,
                    "io.modelcontextprotocol/clientCapabilities": {}
                }
            }),
        )
    };
    let responses = run_stream(
        repository.path(),
        &[
            modern_request(json!("discover"), "server/discover", json!({})),
            unsupported_modern("unknown-modern", "1900-01-01"),
            unsupported_modern("legacy-in-modern", LEGACY_PROTOCOL_VERSION),
            request(
                json!("missing-capabilities"),
                "tools/list",
                json!({
                    "_meta": {
                        "io.modelcontextprotocol/protocolVersion": MODERN_PROTOCOL_VERSION
                    }
                }),
            ),
            request(json!("legacy-before-init"), "tools/list", json!({})),
            request(
                json!("modern-as-legacy-init"),
                "initialize",
                json!({
                    "protocolVersion": MODERN_PROTOCOL_VERSION,
                    "capabilities": {},
                    "clientInfo": {"name": "mixed-client", "version": "1"}
                }),
            ),
            request(json!("discover-without-meta"), "server/discover", json!({})),
        ],
    );

    assert_eq!(responses.len(), 7);
    for response in &responses[1..=2] {
        assert_eq!(response["error"]["code"], -32022);
        assert_eq!(
            response["error"]["data"]["supported"],
            json!([MODERN_PROTOCOL_VERSION])
        );
    }
    assert_eq!(responses[3]["error"]["code"], -32602);
    assert_eq!(responses[4]["error"]["code"], -32602);
    assert_eq!(
        responses[5]["result"]["protocolVersion"],
        LEGACY_PROTOCOL_VERSION
    );
    assert!(responses[5].get("error").is_none());
    assert_eq!(responses[6]["error"]["code"], -32602);
}

#[test]
fn occurred_at_schema_and_runtime_validation_require_rfc3339() {
    let repository = committed_repo();
    let responses = run_stream(
        repository.path(),
        &[
            initialize(json!(1)),
            request(json!(2), "tools/list", json!({})),
            call(
                json!(3),
                "devmap_record_requirement",
                json!({
                    "session_id": "invalid-time-session",
                    "agent_id": "agent-1",
                    "event_id": "invalid-time",
                    "occurred_at": "not-a-timestamp",
                    "source_kind": "human_instruction",
                    "quoted_text": "Timestamp validation is required."
                }),
            ),
            call(
                json!(4),
                "devmap_record_decision",
                json!({
                    "session_id": "invalid-decision-time-session",
                    "agent_id": "agent-1",
                    "occurred_at": "2026-02-30T00:00:00Z",
                    "decision": "Reject an invalid timestamp.",
                    "basis": ["RFC 3339 validation is required."],
                    "alternatives": ["Persist invalid data."],
                    "rationale": "The timestamp cannot identify a real instant.",
                    "scope": "MCP capture",
                    "authority": "Task 7",
                    "revisit_trigger": "The protocol adopts another timestamp format."
                }),
            ),
            call(
                json!(5),
                "devmap_record_evidence",
                json!({
                    "session_id": "invalid-evidence-time-session",
                    "agent_id": "agent-1",
                    "occurred_at": "yesterday",
                    "kind": "test",
                    "target": format!("commit:{}", git(repository.path(), ["rev-parse", "HEAD"])),
                    "outcome": "passed"
                }),
            ),
        ],
    );

    for tool in responses[1]["result"]["tools"].as_array().unwrap() {
        if let Some(occurred_at) = tool["inputSchema"]["properties"].get("occurred_at") {
            assert_eq!(occurred_at["format"], "date-time");
        }
    }
    for response in &responses[2..] {
        assert_eq!(response["result"]["isError"], true);
    }
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    for session_id in [
        "invalid-time-session",
        "invalid-decision-time-session",
        "invalid-evidence-time-session",
    ] {
        assert!(
            JournalStore::open(&workspace, session_id)
                .unwrap()
                .replay()
                .unwrap()
                .is_empty()
        );
    }
}

#[test]
fn concurrent_mcp_processes_persist_every_default_id_call_in_locked_sequence() {
    const CALLS: usize = 12;
    let repository = committed_repo();
    let barrier = Arc::new(Barrier::new(CALLS));
    let mut workers = Vec::new();
    for call_index in 0..CALLS {
        let barrier = Arc::clone(&barrier);
        let source = repository.path().to_path_buf();
        workers.push(thread::spawn(move || {
            barrier.wait();
            let input = [
                initialize(json!(1)),
                call(
                    json!(2),
                    "devmap_record_requirement",
                    json!({
                        "session_id": "concurrent-mcp-session",
                        "agent_id": format!("agent-{call_index}"),
                        "occurred_at": "2026-09-02T12:00:00Z",
                        "source_kind": "human_instruction",
                        "source_locator": format!("call:{call_index}"),
                        "quoted_text": format!("Approved concurrent quotation {call_index}.")
                    }),
                ),
            ]
            .into_iter()
            .map(|message| message.to_string())
            .collect::<Vec<_>>()
            .join("\n");
            let mut child = Command::new(env!("CARGO_BIN_EXE_devmap"))
                .args(["mcp", "--source", source.to_str().unwrap()])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped())
                .spawn()
                .unwrap();
            {
                use std::io::Write;
                writeln!(child.stdin.as_mut().unwrap(), "{input}").unwrap();
            }
            let output = child.wait_with_output().unwrap();
            assert!(output.status.success());
            assert!(output.stderr.is_empty());
            let responses = String::from_utf8(output.stdout)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Value>(line).unwrap())
                .collect::<Vec<_>>();
            assert_eq!(responses.len(), 2);
            assert!(responses[1]["result"].get("isError").is_none());
        }));
    }
    for worker in workers {
        worker.join().unwrap();
    }

    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    let records = JournalStore::open(&workspace, "concurrent-mcp-session")
        .unwrap()
        .replay()
        .unwrap();
    assert_eq!(records.len(), CALLS);
    assert_eq!(
        records
            .iter()
            .map(|record| record.sequence)
            .collect::<Vec<_>>(),
        (1..=CALLS as u64).collect::<Vec<_>>()
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record.event.event_id())
            .collect::<std::collections::BTreeSet<_>>()
            .len(),
        CALLS
    );
    assert_eq!(
        records
            .iter()
            .map(|record| record.event.event_id().to_owned())
            .collect::<std::collections::BTreeSet<_>>(),
        (1..=CALLS)
            .map(|sequence| format!(
                "mcp-devmap_record_requirement-concurrent-mcp-session-{sequence}"
            ))
            .collect()
    );
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
        writeln!(
            stdin,
            "{}",
            modern_request(json!("discover"), "server/discover", json!({}))
        )
        .unwrap();
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
    assert_eq!(messages.len(), 3);
    assert_eq!(messages[0]["result"]["resultType"], "complete");
    assert_eq!(messages[2]["error"]["code"], -32700);
    assert_eq!(messages[2]["id"], Value::Null);
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
