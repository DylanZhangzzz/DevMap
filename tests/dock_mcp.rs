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
    assert_eq!(tools.len(), 6);
    assert_eq!(MCP_TOOLS.len(), 6);
    let snapshot = tools
        .iter()
        .find(|tool| tool["name"] == devmap::mcp::MAP_READ_TOOL)
        .unwrap();
    let render = tools
        .iter()
        .find(|tool| tool["name"] == devmap::mcp::MAP_OPEN_TOOL)
        .unwrap();
    assert_eq!(snapshot["annotations"]["readOnlyHint"], true);
    assert_eq!(
        snapshot["inputSchema"]["properties"]["codex_tasks_complete"]["type"],
        "boolean"
    );
    assert_eq!(
        snapshot["inputSchema"]["properties"]["codex_tasks"]["items"]["properties"]["id"]["pattern"],
        "^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$"
    );
    assert!(snapshot.get("_meta").is_none());
    assert_eq!(render["_meta"]["ui"]["resourceUri"], DOCK_RESOURCE_URI);

    let resource = &responses[2]["result"]["resources"][0];
    assert_eq!(resource["uri"], DOCK_RESOURCE_URI);
    assert_eq!(resource["mimeType"], DOCK_MIME_TYPE);
    let content = &responses[3]["result"]["contents"][0];
    assert_eq!(content["uri"], DOCK_RESOURCE_URI);
    assert_eq!(content["mimeType"], DOCK_MIME_TYPE);
    assert!(
        content["text"]
            .as_str()
            .unwrap()
            .contains("Repository topology")
    );
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
        .find(|tool| tool["name"] == devmap::mcp::MAP_READ_TOOL)
        .unwrap();
    let render = tools
        .iter()
        .find(|tool| tool["name"] == devmap::mcp::MAP_OPEN_TOOL)
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
        "devmap/dock/4"
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
fn data_inventory_refresh_reaches_the_running_viewer() {
    let repo = support::committed_repo();
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();
    let task = json!({"id": "01a00000-0000-7000-8000-000000000001", "title": "Before", "status": "active", "cwd": repo.path().to_string_lossy(), "updatedAt": 1_788_426_685_u64, "hostId": "local", "kind": "codex"});
    let opened = runtime
        .handle(&call(
            json!(2),
            DOCK_BROWSER_TOOL,
            json!({"codex_tasks": [task.clone()]}),
        ))
        .unwrap();
    let url = opened["result"]["structuredContent"]["url"]
        .as_str()
        .unwrap();
    let before = browser_model(url);
    let mut previous = before.clone();
    let mut renamed = task;
    renamed["title"] = json!("After");
    for (index, tool, arguments) in [
        (
            3,
            DOCK_DATA_TOOL,
            json!({"codex_tasks": [renamed.clone()], "codex_tasks_complete": false}),
        ),
        (4, DOCK_RENDER_TOOL, json!({"codex_tasks": [renamed]})),
        (5, DOCK_DATA_TOOL, json!({})),
        (6, DOCK_DATA_TOOL, json!({"codex_tasks": []})),
    ] {
        let response = runtime
            .handle(&call(json!(index), tool, arguments))
            .unwrap();
        assert_ne!(response["result"]["isError"], true);
        let data = &response["result"]["structuredContent"];
        let visible = browser_model(url);
        assert_eq!(visible["lanes"], data["lanes"]);
        assert_eq!(visible["task_observation"], data["task_observation"]);
        assert_ne!(
            visible["task_observation"]["observed_at"],
            before["task_observation"]["observed_at"]
        );
        if index == 5 {
            assert_eq!(visible["task_observation"], previous["task_observation"]);
        } else {
            assert_ne!(
                visible["task_observation"]["observed_at"],
                previous["task_observation"]["observed_at"]
            );
            let (address, query) = url
                .strip_prefix("http://")
                .unwrap()
                .split_once('/')
                .unwrap();
            let events = http_get(
                address.parse().unwrap(),
                &format!(
                    "/api/v1/dock/events{query}&after={}",
                    previous["observation_revision"]
                ),
            );
            let event: Value = serde_json::from_str(
                http_body(&events)
                    .lines()
                    .find_map(|line| line.strip_prefix("data: "))
                    .expect("inventory replacement emits an SSE envelope"),
            )
            .unwrap();
            assert_eq!(event["task_observation"], data["task_observation"]);
            assert_eq!(event["lanes"], data["lanes"]);
        }
        assert_eq!(visible["task_observation"]["complete"], index != 3);
        previous = visible.clone();
        if index == 6 {
            assert_eq!(visible["counts"]["tasks"], 0);
        }
    }
    assert_eq!(runtime.audit().tcp_listeners_opened, 1);
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
                    },
                    {
                        "id": "01a00000-0000-7000-8000-000000000004",
                        "title": "Historical task",
                        "status": "notLoaded",
                        "cwd": repo.path().to_string_lossy(),
                        "updatedAt": 1_788_425_000_u64,
                        "hostId": "local",
                        "kind": "codex"
                    },
                    {
                        "id": "01a00000-0000-7000-8000-000000000005",
                        "title": "Idle task",
                        "status": "idle",
                        "cwd": repo.path().to_string_lossy(),
                        "updatedAt": 1_788_425_500_u64,
                        "hostId": "local",
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
    assert_eq!(chats.len(), 3);
    assert_eq!(
        chats[0]["codex_thread_id"],
        "01a00000-0000-7000-8000-000000000001"
    );
    assert_eq!(chats[0]["display_title"], "修复 DevMap 对话窗口名");
    assert_eq!(chats[0]["host_status"], "active");
    assert!(chats.iter().any(|chat| {
        chat["display_title"] == "Historical task"
            && chat["host_status"] == "notLoaded"
            && chat["status"] == "stale"
    }));
    assert_eq!(chats[0]["association_source"], "codex_task_cwd");
    assert!(!model.to_string().contains("不相关任务"));
    assert!(!model.to_string().contains("远端同路径任务"));
}

#[test]
fn codex_task_inventory_rejects_unsupported_status() {
    let repo = support::committed_repo();
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();
    let response = runtime
        .handle(&call(
            json!(2),
            DOCK_DATA_TOOL,
            json!({
                "codex_tasks": [{
                    "id": "01a00000-0000-7000-8000-000000000004",
                    "title": "Completed task",
                    "status": "completed",
                    "cwd": repo.path().to_string_lossy(),
                    "updatedAt": 1_788_425_000_u64,
                    "hostId": "local",
                    "kind": "codex"
                }]
            }),
        ))
        .unwrap();
    assert_eq!(response["result"]["isError"], true);
    assert!(
        response["result"]["content"][0]["text"]
            .as_str()
            .unwrap()
            .contains("codex_tasks.status")
    );
}

#[test]
fn codex_task_inventory_rejects_ids_that_are_not_host_routable_uuids() {
    let repo = support::committed_repo();
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();
    for invalid_id in [
        "task-one",
        "01a00000-0000-7000-8000-00000000000z",
        "01a00000-0000-7000-8000-000000000001-extra",
    ] {
        let response = runtime
            .handle(&call(
                json!(2),
                DOCK_DATA_TOOL,
                json!({
                    "codex_tasks": [{
                        "id": invalid_id,
                        "title": "Unroutable task",
                        "status": "active",
                        "cwd": repo.path().to_string_lossy(),
                        "updatedAt": 1_788_425_000_u64,
                        "hostId": "local",
                        "kind": "codex"
                    }],
                    "codex_tasks_complete": true
                }),
            ))
            .unwrap();
        assert_eq!(response["result"]["isError"], true, "{invalid_id}");
        assert!(
            response["result"]["content"][0]["text"]
                .as_str()
                .unwrap()
                .contains("codex_tasks.id")
        );
    }
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
            json!({"codex_tasks": [task.clone()], "codex_tasks_complete": true}),
        ))
        .unwrap();
    let url = first["result"]["structuredContent"]["url"]
        .as_str()
        .unwrap();
    let supplied = browser_model(url);
    let supplied_revision = supplied["revision"].as_u64().unwrap();
    let supplied_sync = supplied["task_inventory_synced_at"].clone();
    assert_eq!(supplied["task_observation"]["complete"], true);
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
            json!({"codex_tasks": [task.clone()], "codex_tasks_complete": true}),
        ))
        .unwrap();
    let identical = browser_model(url);
    assert_eq!(identical["revision"], supplied_revision);
    assert_ne!(identical["task_inventory_synced_at"], supplied_sync);
    assert_eq!(identical["task_observation"]["complete"], true);

    runtime
        .handle(&call(
            json!(5),
            DOCK_BROWSER_TOOL,
            json!({"codex_tasks": [task], "codex_tasks_complete": false}),
        ))
        .unwrap();
    let partial = browser_model(url);
    assert!(partial["revision"].as_u64().unwrap() > supplied_revision);
    assert_eq!(partial["task_observation"]["complete"], false);

    runtime
        .handle(&call(
            json!(6),
            DOCK_BROWSER_TOOL,
            json!({"codex_tasks": [], "codex_tasks_complete": true}),
        ))
        .unwrap();
    let cleared = browser_model(url);
    assert!(cleared["revision"].as_u64().unwrap() > supplied_revision);
    assert_ne!(cleared["task_inventory_synced_at"], supplied_sync);
    assert_eq!(cleared["task_observation"]["complete"], true);
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
fn browser_initialization_preserves_copied_unknown_and_fresh_observation_times() {
    let repo = support::committed_repo();
    let task = json!({
        "id": "01a00000-0000-7000-8000-000000000001",
        "title": "Timestamp truth",
        "status": "active",
        "cwd": repo.path().to_string_lossy(),
        "updatedAt": 1_788_426_685_u64,
        "hostId": "local",
        "kind": "codex"
    });

    let mut unknown_runtime = McpRuntime::open(repo.path()).unwrap();
    unknown_runtime.handle(&initialize()).unwrap();
    let unknown = unknown_runtime
        .handle(&call(json!(2), DOCK_BROWSER_TOOL, json!({})))
        .unwrap();
    let unknown_model = browser_model(
        unknown["result"]["structuredContent"]["url"]
            .as_str()
            .unwrap(),
    );
    assert_eq!(
        unknown_model["task_observation"]["observed_at"],
        Value::Null
    );
    assert_eq!(unknown_model["task_observation"]["complete"], false);

    let mut copied_runtime = McpRuntime::open(repo.path()).unwrap();
    copied_runtime.handle(&initialize()).unwrap();
    let observed = copied_runtime
        .handle(&call(
            json!(3),
            DOCK_DATA_TOOL,
            json!({"codex_tasks": [task.clone()], "codex_tasks_complete": false}),
        ))
        .unwrap();
    let original_observed_at =
        observed["result"]["structuredContent"]["task_observation"]["observed_at"].clone();
    std::thread::sleep(Duration::from_millis(10));
    let copied = copied_runtime
        .handle(&call(json!(4), DOCK_BROWSER_TOOL, json!({})))
        .unwrap();
    let copied_url = copied["result"]["structuredContent"]["url"]
        .as_str()
        .unwrap();
    let copied_model = browser_model(copied_url);
    assert_eq!(
        copied_model["task_observation"]["observed_at"],
        original_observed_at
    );
    assert_ne!(
        copied_model["workspace_facts"][0]["git_observed_at"], original_observed_at,
        "preserving task freshness must not backdate a new Git observation"
    );
    assert_eq!(copied_model["task_observation"]["complete"], false);

    std::thread::sleep(Duration::from_millis(10));
    copied_runtime
        .handle(&call(
            json!(5),
            DOCK_BROWSER_TOOL,
            json!({"codex_tasks": [task], "codex_tasks_complete": true}),
        ))
        .unwrap();
    let fresh_model = browser_model(copied_url);
    assert_ne!(
        fresh_model["task_observation"]["observed_at"],
        original_observed_at
    );
    assert_eq!(fresh_model["task_observation"]["complete"], true);
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
fn mcp_opens_an_unborn_repository_and_returns_an_empty_v4_topology() {
    let repo = tempfile::tempdir().unwrap();
    support::git(repo.path(), ["init", "-b", "main"]);
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    runtime.handle(&initialize()).unwrap();

    let response = runtime
        .handle(&call(json!(2), DOCK_DATA_TOOL, json!({})))
        .unwrap();
    let model = &response["result"]["structuredContent"];

    assert_eq!(model["schema_version"], "devmap/dock/4");
    assert_eq!(model["topology"]["commits"], json!([]));
    assert_eq!(model["task_observation"]["complete"], false);
    assert_eq!(model["task_observation"]["observed_at"], Value::Null);
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
