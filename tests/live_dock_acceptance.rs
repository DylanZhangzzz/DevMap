mod support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{Cursor, Read, Write};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, TcpStream};
use std::time::{Duration, Instant};

use devmap::canonical::canonical_json;
use devmap::cli::{AdapterHost, HookHandleArgs};
use devmap::dock::{DockReducer, DockService, NoRoutes};
use devmap::git::SourceGitInspector;
use devmap::hook::handle_hook;
use devmap::journal::{JournalIntegrity, JournalSummary};
use devmap::mcp::{DOCK_DATA_TOOL, McpRuntime};
use devmap::presence::PresenceStatus;
use devmap::viewer::start_live_viewer;
use serde_json::{Value, json};

const CANARIES: [&str; 6] = [
    "PROMPT_CANARY_91D2",
    "COMMAND_CANARY_91D2",
    "PATCH_CANARY_91D2",
    "TOOL_INPUT_CANARY_91D2",
    "TOOL_OUTPUT_CANARY_91D2",
    "TRANSCRIPT_CANARY_91D2",
];

fn native_event(host: AdapterHost, name: &str, session_id: &str) -> Value {
    let raw = match host {
        AdapterHost::Codex => include_str!("fixtures/hooks/codex-events.json"),
        AdapterHost::Claude => include_str!("fixtures/hooks/claude-events.json"),
        AdapterHost::GenericMcp => unreachable!(),
    };
    let mut event = serde_json::from_str::<BTreeMap<String, Value>>(raw).unwrap()[name].clone();
    event["session_id"] = json!(session_id);
    event["transcript_path"] = json!(CANARIES[5]);
    event["prompt"] = json!(CANARIES[0]);
    event["patch"] = json!(CANARIES[2]);
    event["tool_input"] = json!({"command": CANARIES[1], "raw": CANARIES[3]});
    event["tool_response"] = json!({"output": CANARIES[4]});
    event
}

fn hook(source: &std::path::Path, host: AdapterHost, name: &str, session_id: &str) {
    let mut input = Cursor::new(serde_json::to_vec(&native_event(host, name, session_id)).unwrap());
    handle_hook(
        HookHandleArgs {
            source: source.to_path_buf(),
            host,
            event: name.into(),
        },
        &mut input,
    )
    .unwrap();
}

fn mcp_request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn http_get(address: SocketAddr, target: &str) -> String {
    let mut stream = TcpStream::connect(address).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    write!(
        stream,
        "GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n"
    )
    .unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    response
}

fn http_body(response: &str) -> &str {
    response.split_once("\r\n\r\n").unwrap().1
}

#[test]
fn shared_model_unifies_three_hosts_without_leaking_or_mutating_source_git() {
    let fixture = support::live_dock_fixture();
    let before = support::source_snapshot(fixture.repo.path());

    hook(
        fixture.agent_a.path(),
        AdapterHost::Codex,
        "SessionStart",
        "codex-live",
    );
    hook(
        fixture.agent_a.path(),
        AdapterHost::Codex,
        "PostToolUse",
        "codex-live",
    );
    hook(
        fixture.agent_b.path(),
        AdapterHost::Claude,
        "SessionStart",
        "claude-done",
    );
    hook(
        fixture.agent_b.path(),
        AdapterHost::Claude,
        "SessionEnd",
        "claude-done",
    );

    let mut generic = McpRuntime::open(fixture.agent_b.path()).unwrap();
    let initialized = generic
        .handle(&mcp_request(
            0,
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "acceptance", "version": "1"}
            }),
        ))
        .unwrap();
    assert!(initialized["result"]["serverInfo"].is_object());
    let decision = generic
        .handle(&mcp_request(
            1,
            "tools/call",
            json!({
                "name": "devmap_record_decision",
                "arguments": {
                    "session_id": "generic-stale",
                    "agent_id": "generic-agent",
                    "event_id": "generic-decision",
                    "occurred_at": "2026-09-02T12:00:00Z",
                    "decision": "Use the host-neutral event contract.",
                    "basis": ["The adapter contract is shared."],
                    "alternatives": ["A host-specific model"],
                    "rationale": "One read model can represent all supported hosts.",
                    "scope": "Live Dock acceptance",
                    "authority": "approved implementation plan",
                    "revisit_trigger": "The host-neutral contract changes."
                }
            }),
        ))
        .unwrap();
    assert_ne!(decision["result"]["isError"], true, "{decision}");

    let workspace_b = SourceGitInspector::open(fixture.agent_b.path())
        .unwrap()
        .workspace()
        .unwrap();
    let presence_root = workspace_b.git_common_dir.join("devmap/presence/v1");
    let stale_path = presence_root.join("generic-stale.json");
    let mut stale: Value = serde_json::from_slice(&fs::read(&stale_path).unwrap()).unwrap();
    stale["lease_expires_at"] = json!("2020-01-01T00:00:00Z");
    fs::write(&stale_path, canonical_json(&stale).unwrap()).unwrap();
    fs::write(presence_root.join("corrupt-record.json"), b"not-json").unwrap();

    let workspace_a = SourceGitInspector::open(fixture.agent_a.path())
        .unwrap()
        .workspace()
        .unwrap();
    let codex_events = workspace_a
        .git_dir
        .join("devmap/sessions/codex-live/events.ndjson");
    let mut events = fs::OpenOptions::new()
        .append(true)
        .open(codex_events)
        .unwrap();
    events.write_all(b"corrupt-tail\n").unwrap();

    let mut service = DockService::open(fixture.repo.path()).unwrap();
    service.refresh(time::OffsetDateTime::now_utc()).unwrap();
    let model = service.snapshot();
    let all = model
        .current
        .iter()
        .chain(&model.active)
        .chain(&model.stale_or_uninstrumented)
        .collect::<Vec<_>>();
    assert_eq!(model.current.len(), 1);
    assert_eq!(model.current[0].status, PresenceStatus::Unknown);
    assert!(
        all.iter().any(
            |row| row.host.as_deref() == Some("codex") && row.status == PresenceStatus::Working
        )
    );
    assert!(all.iter().any(
        |row| row.host.as_deref() == Some("claude") && row.status == PresenceStatus::Completed
    ));
    assert!(all.iter().any(
        |row| row.host.as_deref() == Some("generic_mcp") && row.status == PresenceStatus::Stale
    ));
    assert!(all.iter().all(|row| row.route_id.is_none()));
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "presence_record_invalid")
    );
    assert!(
        model
            .warnings
            .iter()
            .any(|warning| warning.code == "journal_corrupt")
    );
    assert!(all.iter().any(|row| row.capture_incomplete));

    let agents_json = devmap::run([
        "devmap",
        "agents",
        "--source",
        fixture.repo.path().to_str().unwrap(),
        "--json",
    ])
    .unwrap()
    .stdout;
    let mut dock_mcp = McpRuntime::open(fixture.repo.path()).unwrap();
    let mcp_snapshot = dock_mcp
        .handle(&mcp_request(
            2,
            "tools/call",
            json!({"name": DOCK_DATA_TOOL, "arguments": {}}),
        ))
        .unwrap()
        .to_string();
    let mcp_html = dock_mcp
        .handle(&mcp_request(
            3,
            "resources/read",
            json!({"uri": devmap::dock_asset::DOCK_RESOURCE_URI}),
        ))
        .unwrap()
        .to_string();
    let (viewer, runtime) = start_live_viewer(
        fixture.repo.path(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
    )
    .unwrap();
    let http_snapshot = http_get(
        viewer.address,
        &format!("/api/v1/dock/snapshot?token={}", viewer.token),
    );
    let sse = http_get(
        viewer.address,
        &format!("/api/v1/dock/events?token={}&after=0", viewer.token),
    );
    runtime.shutdown().unwrap();

    let presence = fs::read_dir(&presence_root)
        .unwrap()
        .filter_map(Result::ok)
        .filter_map(|entry| fs::read(entry.path()).ok())
        .flatten()
        .collect::<Vec<_>>();
    let outputs = format!(
        "{}{}{}{}{}{}",
        String::from_utf8_lossy(&presence),
        agents_json,
        mcp_snapshot,
        mcp_html,
        http_snapshot,
        sse
    );
    for canary in CANARIES {
        assert!(!outputs.contains(canary), "leaked {canary}");
    }
    assert_eq!(
        support::source_snapshot(fixture.repo.path()).git,
        before.git
    );
}

#[test]
fn bounded_large_reduction_and_live_revision_meet_mvp_latency_targets() {
    let fixture = support::dock_reducer_fixture();
    let template = fixture.presence.records[0].clone();
    let mut worktrees = Vec::new();
    let mut records = Vec::new();
    let mut journals = BTreeMap::new();
    for worktree_index in 0..100 {
        let mut worktree = fixture.worktrees[0].clone();
        worktree.worktree_id = format!("wt-{worktree_index:064x}");
        worktree.is_current = worktree_index == 0;
        worktrees.push(worktree.clone());
        for agent_index in 0..10 {
            let mut record = template.clone();
            record.worktree_id = worktree.worktree_id.clone();
            record.session_id = format!("perf-{worktree_index:03}-{agent_index:02}");
            record.actor_id = format!("agent-{worktree_index:03}-{agent_index:02}");
            journals.insert(
                record.session_id.clone(),
                JournalSummary {
                    session_id: record.session_id.clone(),
                    records: 1,
                    last_sequence: Some(1),
                    last_sha256: Some("a".repeat(64)),
                    integrity: JournalIntegrity::Verified,
                },
            );
            records.push(record);
        }
    }
    let started = Instant::now();
    let model = DockReducer::new(NoRoutes)
        .reduce(
            &fixture.workspace,
            worktrees,
            devmap::presence::PresenceLoadReport {
                records,
                warnings: vec![],
                truncated: false,
            },
            journals,
            fixture.now,
        )
        .unwrap();
    let reduction = started.elapsed();
    println!("100 worktrees / 1000 Presence reduction: {reduction:?}");
    assert!(reduction < Duration::from_secs(1));
    assert_eq!(model.current.len(), 10);

    let repo = support::committed_repo();
    let (viewer, runtime) = start_live_viewer(
        repo.path(),
        SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0),
    )
    .unwrap();
    let first = http_get(
        viewer.address,
        &format!("/api/v1/dock/snapshot?token={}", viewer.token),
    );
    let revision = serde_json::from_str::<Value>(http_body(&first)).unwrap()["revision"]
        .as_u64()
        .unwrap();
    let update_started = Instant::now();
    let _other = support::linked_worktree(repo.path(), "codex/perf-visible");
    std::thread::sleep(Duration::from_millis(550));
    let update = http_get(
        viewer.address,
        &format!(
            "/api/v1/dock/events?token={}&after={revision}",
            viewer.token
        ),
    );
    let visible = update_started.elapsed();
    println!("Git change to SSE-visible revision: {visible:?}");
    assert!(http_body(&update).contains("event: dock"));
    assert!(visible < Duration::from_secs(2));
    runtime.shutdown().unwrap();
}
