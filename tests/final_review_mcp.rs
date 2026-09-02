mod support;

use std::io::Cursor;

use devmap::git::SourceGitInspector;
use devmap::journal::JournalStore;
use devmap::mcp::serve_mcp;
use serde_json::{Value, json};
use support::committed_repo;

const LEGACY: &str = "2025-11-25";
const MODERN: &str = "2026-07-28";

fn modern_request(id: Value, method: &str, mut params: Value) -> Value {
    params.as_object_mut().unwrap().insert(
        "_meta".into(),
        json!({
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientInfo": {
                "name": "final-review",
                "version": "1.0.0"
            },
            "io.modelcontextprotocol/clientCapabilities": {
                "roots": {},
                "sampling": {"tools": {}},
                "elicitation": {"form": {}, "url": {}},
                "extensions": {"io.modelcontextprotocol/example": {}}
            }
        }),
    );
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn run(source: &std::path::Path, messages: &[Value]) -> Vec<Value> {
    let input = messages
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    run_bytes(source, input.into_bytes())
}

fn run_bytes(source: &std::path::Path, input: Vec<u8>) -> Vec<Value> {
    let mut output = Vec::new();
    serve_mcp(source, Cursor::new(input), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect()
}

#[test]
fn modern_discovery_and_version_errors_never_advertise_legacy_versions() {
    let repository = committed_repo();
    let mut unsupported = modern_request(json!(2), "tools/list", json!({}));
    unsupported["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] = json!("2099-01-01");
    let responses = run(
        repository.path(),
        &[
            modern_request(json!(1), "server/discover", json!({})),
            unsupported,
        ],
    );

    assert_eq!(responses[0]["result"]["supportedVersions"], json!([MODERN]));
    assert_eq!(responses[1]["error"]["code"], -32022);
    assert_eq!(responses[1]["error"]["data"]["supported"], json!([MODERN]));
}

#[test]
fn unsupported_legacy_initialize_is_a_successful_counteroffer() {
    let repository = committed_repo();
    let responses = run(
        repository.path(),
        &[
            json!({
                "jsonrpc": "2.0",
                "id": "counteroffer",
                "method": "initialize",
                "params": {
                    "protocolVersion": "2024-01-01",
                    "capabilities": {"roots": {"listChanged": true}},
                    "clientInfo": {"name": "legacy-client", "version": "1.2.3"}
                }
            }),
            json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
        ],
    );

    assert_eq!(responses[0]["result"]["protocolVersion"], LEGACY);
    assert!(responses[0].get("error").is_none());
    assert!(responses[1]["result"]["tools"].is_array());
}

#[test]
fn modern_metadata_validates_client_identity_and_nested_capability_shapes() {
    let repository = committed_repo();
    let mut optional_client = modern_request(json!(1), "tools/list", json!({}));
    optional_client["params"]["_meta"]
        .as_object_mut()
        .unwrap()
        .remove("io.modelcontextprotocol/clientInfo");
    let mut bad_identity = modern_request(json!(2), "tools/list", json!({}));
    bad_identity["params"]["_meta"]["io.modelcontextprotocol/clientInfo"] =
        json!({"name": "client"});
    let mut bad_icon = modern_request(json!(3), "tools/list", json!({}));
    bad_icon["params"]["_meta"]["io.modelcontextprotocol/clientInfo"] = json!({
        "name": "client",
        "version": "1.0.0",
        "icons": [{"src": "https://example.test/icon.png", "sizes": "48x48"}]
    });
    let mut bad_sampling = modern_request(json!(4), "tools/list", json!({}));
    bad_sampling["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] =
        json!({"sampling": {"tools": true}});
    let mut bad_extensions = modern_request(json!(5), "tools/list", json!({}));
    bad_extensions["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] =
        json!({"extensions": {"io.modelcontextprotocol/example": "yes"}});
    let mut unprefixed_extension = modern_request(json!(6), "tools/list", json!({}));
    unprefixed_extension["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"] =
        json!({"extensions": {"example": {}}});
    let responses = run(
        repository.path(),
        &[
            optional_client,
            bad_identity,
            bad_icon,
            bad_sampling,
            bad_extensions,
            unprefixed_extension,
        ],
    );

    assert!(responses[0]["result"]["tools"].is_array());
    assert!(
        responses[1..]
            .iter()
            .all(|response| response["error"]["code"] == -32602)
    );
}

#[test]
fn mcp_line_limit_is_enforced_before_json_parsing_and_next_line_still_works() {
    let repository = committed_repo();
    let mut input = vec![b' '; devmap::mcp::MAX_MCP_LINE_BYTES + 1];
    input.push(b'\n');
    input.extend_from_slice(
        (modern_request(json!(2), "server/discover", json!({})).to_string() + "\n").as_bytes(),
    );

    let responses = run_bytes(repository.path(), input);

    assert_eq!(responses.len(), 2);
    assert_eq!(responses[0]["error"]["code"], -32600);
    assert_eq!(responses[0]["error"]["data"]["resource"], "MCP line");
    assert_eq!(responses[1]["result"]["supportedVersions"], json!([MODERN]));
}

#[test]
fn modern_metadata_and_semantic_arguments_have_independent_bounds() {
    let repository = committed_repo();
    let mut huge_metadata = modern_request(json!(1), "tools/list", json!({}));
    huge_metadata["params"]["_meta"]["extra"] =
        json!("x".repeat(devmap::mcp::MAX_MCP_METADATA_BYTES));
    let long_semantic = modern_request(
        json!(2),
        "tools/call",
        json!({
            "name": "devmap_record_requirement",
            "arguments": {
                "session_id": "bounded-session",
                "agent_id": "agent",
                "source_kind": "human_instruction",
                "quoted_text": "x".repeat(devmap::mcp::MAX_SEMANTIC_STRING_BYTES + 1)
            }
        }),
    );
    let too_many_basis = modern_request(
        json!(3),
        "tools/call",
        json!({
            "name": "devmap_record_decision",
            "arguments": {
                "session_id": "bounded-session",
                "agent_id": "agent",
                "decision": "Bound requests.",
                "basis": (0..=devmap::mcp::MAX_SEMANTIC_ARRAY_ITEMS).map(|i| format!("basis {i}")).collect::<Vec<_>>(),
                "alternatives": ["one"],
                "rationale": "bounded",
                "scope": "MCP",
                "authority": "review",
                "revisit_trigger": "new limits"
            }
        }),
    );
    let responses = run(
        repository.path(),
        &[huge_metadata, long_semantic, too_many_basis],
    );

    assert_eq!(responses[0]["error"]["code"], -32602, "{responses:#?}");
    assert_eq!(responses[1]["result"]["isError"], true, "{responses:#?}");
    assert_eq!(responses[2]["result"]["isError"], true, "{responses:#?}");
    for response in &responses[1..] {
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("resource limit exceeded"),
            "{responses:#?}"
        );
    }
    let workspace = SourceGitInspector::open(repository.path())
        .unwrap()
        .workspace()
        .unwrap();
    assert!(
        JournalStore::open(&workspace, "bounded-session")
            .unwrap()
            .replay()
            .unwrap()
            .is_empty()
    );
}

#[test]
fn generic_context_reports_capability_derived_grade_d() {
    let repository = committed_repo();
    let response = run(
        repository.path(),
        &[modern_request(
            json!(1),
            "tools/call",
            json!({"name": "devmap_context", "arguments": {}}),
        )],
    );

    assert_eq!(
        response[0]["result"]["structuredContent"]["capture_grade"],
        "D"
    );
}
