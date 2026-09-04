mod support;

use std::io::{Cursor, Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use devmap::dock_asset::{DOCK_MIME_TYPE, DOCK_RESOURCE_URI};
use devmap::mcp::{
    DOCK_BROWSER_TOOL, DOCK_DATA_TOOL, DOCK_RENDER_TOOL, MCP_TOOLS, McpRuntime, serve_mcp,
};
use serde_json::{Value, json};

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

fn http_get(address: SocketAddr, target: &str) -> String {
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(2)).unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: {address}\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    stream.flush().unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn http_body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}

fn browser_model(url: &str) -> Value {
    let target = url.strip_prefix("http://").unwrap();
    let (address, query) = target.split_once('/').unwrap();
    let address: SocketAddr = address.parse().unwrap();
    let response = http_get(address, &format!("/api/v1/dock/snapshot{query}"));
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    serde_json::from_str(http_body(&response)).unwrap()
}

fn run_stream(source: &std::path::Path, messages: &[Value]) -> Vec<Value> {
    let mut input = messages
        .iter()
        .map(Value::to_string)
        .collect::<Vec<_>>()
        .join("\n");
    input.push('\n');
    let mut output = Vec::new();
    serve_mcp(source, Cursor::new(input.into_bytes()), &mut output).unwrap();
    String::from_utf8(output)
        .unwrap()
        .lines()
        .map(|line| {
            assert!(line.len() <= devmap::mcp::MAX_MCP_LINE_BYTES);
            serde_json::from_str(line).unwrap()
        })
        .collect()
}

fn initialize() -> Value {
    request(
        json!(1),
        "initialize",
        json!({
            "protocolVersion": "2025-11-25",
            "capabilities": {},
            "clientInfo": {"name": "dock-test", "version": "1"}
        }),
    )
}

#[test]
fn dock_resource_and_decoupled_tools_are_advertised() {
    let repo = support::committed_repo();
    let responses = run_stream(
        repo.path(),
        &[
            initialize(),
            request(json!(2), "tools/list", json!({})),
            request(json!(3), "resources/list", json!({})),
            request(
                json!(4),
                "resources/read",
                json!({"uri": DOCK_RESOURCE_URI}),
            ),
        ],
    );

    assert_eq!(
        responses[0]["result"]["capabilities"],
        json!({"resources": {}, "tools": {}})
    );
    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 7);
    assert_eq!(MCP_TOOLS.len(), 7);
    let snapshot = tools
        .iter()
        .find(|tool| tool["name"] == DOCK_DATA_TOOL)
        .unwrap();
    let render = tools
        .iter()
        .find(|tool| tool["name"] == DOCK_RENDER_TOOL)
        .unwrap();
    assert_eq!(snapshot["annotations"]["readOnlyHint"], true);
    assert!(snapshot.get("_meta").is_none());
    assert_eq!(render["_meta"]["ui"]["resourceUri"], DOCK_RESOURCE_URI);

    let resource = &responses[2]["result"]["resources"][0];
    assert_eq!(resource["uri"], DOCK_RESOURCE_URI);
    assert_eq!(resource["mimeType"], DOCK_MIME_TYPE);
    let content = &responses[3]["result"]["contents"][0];
    assert_eq!(content["uri"], DOCK_RESOURCE_URI);
    assert_eq!(content["mimeType"], DOCK_MIME_TYPE);
    assert!(content["text"].as_str().unwrap().contains("Git Work Map"));
    assert_eq!(content["_meta"]["ui"]["csp"]["connectDomains"], json!([]));
}

#[test]
fn render_tool_advertises_the_openai_output_template_compatibility_alias() {
    let repo = support::committed_repo();
    let responses = run_stream(
        repo.path(),
        &[initialize(), request(json!(2), "tools/list", json!({}))],
    );

    let tools = responses[1]["result"]["tools"].as_array().unwrap();
    let snapshot = tools
        .iter()
        .find(|tool| tool["name"] == DOCK_DATA_TOOL)
        .unwrap();
    let render = tools
        .iter()
        .find(|tool| tool["name"] == DOCK_RENDER_TOOL)
        .unwrap();

    assert!(snapshot["_meta"].get("openai/outputTemplate").is_none());
    assert_eq!(
        render["_meta"]["openai/outputTemplate"],
        "ui://devmap/dock/v1.html"
    );
}

#[test]
fn dock_calls_are_read_only_closed_world_and_revisioned() {
    let repo = support::committed_repo();
    let before = support::source_snapshot(repo.path());
    let responses = run_stream(
        repo.path(),
        &[
            initialize(),
            call(json!(2), DOCK_DATA_TOOL, json!({})),
            call(json!(3), DOCK_DATA_TOOL, json!({})),
            call(json!(4), DOCK_RENDER_TOOL, json!({})),
            call(json!(5), DOCK_DATA_TOOL, json!({"unexpected": true})),
            request(json!(6), "resources/read", json!({"uri": "ui://other"})),
        ],
    );

    let revisions = responses[1..=3]
        .iter()
        .map(|response| {
            response["result"]["structuredContent"]["revision"]
                .as_u64()
                .unwrap()
        })
        .collect::<Vec<_>>();
    assert!(revisions.windows(2).all(|pair| pair[0] <= pair[1]));
    assert_eq!(
        responses[1]["result"]["structuredContent"]["schema_version"],
        "devmap/dock/3"
    );
    assert!(responses[1]["result"].get("_meta").is_none());
    assert_eq!(
        responses[3]["result"]["_meta"]["ui"]["resourceUri"],
        DOCK_RESOURCE_URI
    );
    assert_eq!(responses[4]["result"]["isError"], true);
    assert_eq!(responses[5]["error"]["code"], -32602);
    assert_eq!(support::source_snapshot(repo.path()), before);
}

#[test]
fn mcp_runtime_audit_never_reports_a_tcp_listener() {
    let repo = support::committed_repo();
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    assert!(runtime.handle(&initialize()).unwrap()["result"].is_object());
    for id in 1..=3 {
        let response = runtime
            .handle(&call(json!(id), DOCK_DATA_TOOL, json!({})))
            .unwrap();
        assert!(response["result"]["structuredContent"]["revision"].is_u64());
    }
    assert_eq!(runtime.audit().stdio_messages, 4);
    assert_eq!(runtime.audit().tcp_listeners_opened, 0);
}

#[test]
fn browser_tool_is_the_only_dock_tool_that_opens_and_reuses_a_listener() {
    let repo = support::committed_repo();
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();
    runtime
        .handle(&call(json!(2), DOCK_DATA_TOOL, json!({})))
        .unwrap();
    runtime
        .handle(&call(json!(3), DOCK_RENDER_TOOL, json!({})))
        .unwrap();
    assert_eq!(runtime.audit().tcp_listeners_opened, 0);

    let first = runtime
        .handle(&call(json!(4), DOCK_BROWSER_TOOL, json!({})))
        .unwrap();
    let second = runtime
        .handle(&call(json!(5), DOCK_BROWSER_TOOL, json!({})))
        .unwrap();

    assert_eq!(runtime.audit().tcp_listeners_opened, 1);
    assert_eq!(
        first["result"]["structuredContent"]["url"],
        second["result"]["structuredContent"]["url"]
    );
    assert_eq!(first["result"]["structuredContent"]["reused"], false);
    assert_eq!(second["result"]["structuredContent"]["reused"], true);
    assert!(first["result"]["structuredContent"]["revision"].is_u64());
    assert!(
        !first["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("token=")
    );
}

#[test]
fn browser_dock_projects_exact_codex_task_titles_into_their_workspace() {
    let repo = support::committed_repo();
    let unrelated = tempfile::tempdir().unwrap();
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();
    let response = runtime
        .handle(&call(
            json!(2),
            DOCK_BROWSER_TOOL,
            json!({
                "codex_tasks": [
                    {
                        "id": "01a00000-0000-7000-8000-000000000001",
                        "title": "修复 DevMap 对话窗口名",
                        "status": "active",
                        "cwd": repo.path().to_string_lossy(),
                        "updatedAt": 1_788_426_685_u64,
                        "hostId": "local",
                        "kind": "codex"
                    },
                    {
                        "id": "01a00000-0000-7000-8000-000000000002",
                        "title": "不相关任务",
                        "status": "idle",
                        "cwd": unrelated.path().to_string_lossy(),
                        "updatedAt": 1_788_426_000_u64,
                        "hostId": "local",
                        "kind": "codex"
                    },
                    {
                        "id": "01a00000-0000-7000-8000-000000000003",
                        "title": "远端同路径任务",
                        "status": "active",
                        "cwd": repo.path().to_string_lossy(),
                        "updatedAt": 1_788_426_500_u64,
                        "hostId": "remote-host",
                        "kind": "codex"
                    }
                ]
            }),
        ))
        .unwrap();

    assert_ne!(response["result"]["isError"], true);
    let url = response["result"]["structuredContent"]["url"]
        .as_str()
        .unwrap();
    let target = url.strip_prefix("http://").unwrap();
    let (address, query) = target.split_once('/').unwrap();
    let address: SocketAddr = address.parse().unwrap();
    let response = http_get(address, &format!("/api/v1/dock/snapshot{query}"));
    assert!(response.starts_with("HTTP/1.1 200"), "{response}");
    let model: Value = serde_json::from_str(http_body(&response)).unwrap();
    let chats = model["lanes"][0]["chats"].as_array().unwrap();
    assert_eq!(chats.len(), 1);
    assert_eq!(chats[0]["display_title"], "修复 DevMap 对话窗口名");
    assert_eq!(chats[0]["host_status"], "active");
    assert_eq!(chats[0]["association_source"], "codex_task_cwd");
    assert!(!model.to_string().contains("不相关任务"));
    assert!(!model.to_string().contains("远端同路径任务"));
}

#[test]
fn codex_task_inventory_distinguishes_omit_replace_and_clear() {
    let repo = support::committed_repo();
    let task = json!({
        "id": "01a00000-0000-7000-8000-000000000001",
        "title": "Current task title",
        "status": "active",
        "cwd": repo.path().to_string_lossy(),
        "updatedAt": 1_788_426_685_u64,
        "hostId": "local",
        "kind": "codex"
    });
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();
    let first = runtime
        .handle(&call(
            json!(2),
            DOCK_BROWSER_TOOL,
            json!({"codex_tasks": [task.clone()]}),
        ))
        .unwrap();
    let url = first["result"]["structuredContent"]["url"]
        .as_str()
        .unwrap();
    let supplied = browser_model(url);
    let supplied_revision = supplied["revision"].as_u64().unwrap();
    let supplied_sync = supplied["task_inventory_synced_at"].clone();
    assert_eq!(
        supplied["branch_groups"][0]["lanes"][0]["chats"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    runtime
        .handle(&call(json!(3), DOCK_BROWSER_TOOL, json!({})))
        .unwrap();
    let retained = browser_model(url);
    assert_eq!(retained["revision"], supplied_revision);
    assert_eq!(retained["task_inventory_synced_at"], supplied_sync);
    assert_eq!(
        retained["branch_groups"][0]["lanes"][0]["chats"]
            .as_array()
            .unwrap()
            .len(),
        1
    );

    runtime
        .handle(&call(
            json!(4),
            DOCK_BROWSER_TOOL,
            json!({"codex_tasks": [task]}),
        ))
        .unwrap();
    let identical = browser_model(url);
    assert_eq!(identical["revision"], supplied_revision);
    assert_eq!(identical["task_inventory_synced_at"], supplied_sync);

    runtime
        .handle(&call(
            json!(5),
            DOCK_BROWSER_TOOL,
            json!({"codex_tasks": []}),
        ))
        .unwrap();
    let cleared = browser_model(url);
    assert!(cleared["revision"].as_u64().unwrap() > supplied_revision);
    assert_ne!(cleared["task_inventory_synced_at"], supplied_sync);
    assert!(
        cleared["branch_groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["lanes"].as_array().unwrap())
            .all(|lane| lane["chats"].as_array().unwrap().is_empty())
    );
}

#[test]
fn starting_browser_with_empty_inventory_clears_retained_dock_tasks() {
    let repo = support::committed_repo();
    let task = json!({
        "id": "01a00000-0000-7000-8000-000000000001",
        "title": "Task that must be cleared",
        "status": "active",
        "cwd": repo.path().to_string_lossy(),
        "updatedAt": 1_788_426_685_u64,
        "hostId": "local",
        "kind": "codex"
    });
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();
    runtime
        .handle(&call(
            json!(2),
            DOCK_DATA_TOOL,
            json!({"codex_tasks": [task]}),
        ))
        .unwrap();

    runtime
        .handle(&call(
            json!(3),
            DOCK_BROWSER_TOOL,
            json!({"codex_tasks": []}),
        ))
        .unwrap();
    let retained = runtime
        .handle(&call(json!(4), DOCK_DATA_TOOL, json!({})))
        .unwrap();
    let model = &retained["result"]["structuredContent"];

    assert!(
        model["branch_groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|group| group["lanes"].as_array().unwrap())
            .all(|lane| lane["chats"].as_array().unwrap().is_empty())
    );
}

#[test]
fn dropping_mcp_runtime_stops_the_browser_dock_listener() {
    let repo = support::committed_repo();
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();
    let response = runtime
        .handle(&call(json!(2), DOCK_BROWSER_TOOL, json!({})))
        .unwrap();
    let url = response["result"]["structuredContent"]["url"]
        .as_str()
        .unwrap();
    let address = url
        .strip_prefix("http://")
        .unwrap()
        .split_once('/')
        .unwrap()
        .0
        .parse()
        .unwrap();
    assert!(TcpStream::connect_timeout(&address, Duration::from_secs(1)).is_ok());

    drop(runtime);

    let deadline = Instant::now() + Duration::from_secs(1);
    while Instant::now() < deadline
        && TcpStream::connect_timeout(&address, Duration::from_millis(50)).is_ok()
    {}
    assert!(TcpStream::connect_timeout(&address, Duration::from_millis(100)).is_err());
}

#[test]
fn semantic_capture_starts_even_when_the_optional_dock_inventory_is_broken() {
    let repo = support::committed_repo();
    let presence = repo.path().join(".git/devmap/presence");
    std::fs::create_dir_all(&presence).unwrap();
    std::fs::write(presence.join("v1"), b"not-a-directory").unwrap();
    let mut runtime = McpRuntime::open(repo.path()).expect("MCP capture must not require the Dock");
    assert!(runtime.handle(&initialize()).unwrap()["result"].is_object());
    let context = runtime
        .handle(&call(json!(2), "devmap_context", json!({})))
        .unwrap();
    assert_ne!(context["result"]["isError"], true);
    let dock = runtime
        .handle(&call(json!(3), DOCK_DATA_TOOL, json!({})))
        .unwrap();
    assert_eq!(dock["result"]["isError"], true);
}

#[test]
fn resource_listing_rejects_unknown_fields() {
    let repo = support::committed_repo();
    let responses = run_stream(
        repo.path(),
        &[
            initialize(),
            request(json!(2), "resources/list", json!({"unexpected": true})),
        ],
    );
    assert_eq!(responses[1]["error"]["code"], -32602);
}
