mod support;

use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

const PLUGIN_ROOT: &str = "plugins/devmap";

fn read_json(path: impl AsRef<Path>) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

#[test]
fn plugin_manifest_and_stdio_policy_are_minimal_and_portable() {
    let manifest = read_json(format!("{PLUGIN_ROOT}/.codex-plugin/plugin.json"));
    let mcp = read_json(format!("{PLUGIN_ROOT}/.mcp.json"));

    assert_eq!(manifest["name"], "devmap");
    assert_eq!(manifest["mcpServers"], "./.mcp.json");
    assert_eq!(manifest["skills"], "./skills/");
    assert!(manifest["author"]["name"].is_string());
    assert_eq!(manifest["interface"]["displayName"], "DevMap");
    assert_eq!(
        manifest["interface"]["defaultPrompt"],
        "Open the DevMap Git relationship map on the right."
    );

    let servers = mcp["mcpServers"].as_object().unwrap();
    assert_eq!(servers.len(), 1);
    let server = &servers["devmap"];
    assert_eq!(server["command"], "devmap");
    assert_eq!(server["args"], json!(["mcp"]));
    assert_eq!(server["enabled"], true);
    assert_eq!(server["startup_timeout_sec"], 10);
    assert_eq!(server["tool_timeout_sec"], 10);
    assert_eq!(server["default_tools_approval_mode"], "writes");
    assert_eq!(server["tools"].as_object().unwrap().len(), 3);
    assert_eq!(
        server["tools"]["devmap_dock_snapshot"]["approval_mode"],
        "auto"
    );
    assert_eq!(server["tools"]["devmap_open_dock"]["approval_mode"], "auto");
    assert_eq!(
        server["tools"]["devmap_start_browser_dock"]["approval_mode"],
        "auto"
    );
    assert!(server.get("cwd").is_none());
    assert!(server.get("url").is_none());
}

#[test]
fn configured_command_launches_dock_over_stdio_without_browser_server() {
    let repo = support::committed_repo();
    let mcp = read_json(format!("{PLUGIN_ROOT}/.mcp.json"));
    let server = &mcp["mcpServers"]["devmap"];
    let binary = Path::new(env!("CARGO_BIN_EXE_devmap"));
    let binary_dir = binary.parent().unwrap();
    let mut paths = vec![binary_dir.to_path_buf()];
    paths.extend(std::env::split_paths(
        &std::env::var_os("PATH").unwrap_or_default(),
    ));
    let path = std::env::join_paths(paths).unwrap();
    let mut child = Command::new(server["command"].as_str().unwrap())
        .args(
            server["args"]
                .as_array()
                .unwrap()
                .iter()
                .map(|arg| arg.as_str().unwrap()),
        )
        .current_dir(repo.path())
        .env("PATH", path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let messages = [
        json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "plugin-launch-test", "version": "1"}
            }
        }),
        json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}}),
    ];
    let stdin = child.stdin.as_mut().unwrap();
    for message in messages {
        writeln!(stdin, "{message}").unwrap();
    }
    drop(child.stdin.take());
    let output = child.wait_with_output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let responses = String::from_utf8(output.stdout)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str::<Value>(line).unwrap())
        .collect::<Vec<_>>();
    let names = responses[1]["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"devmap_dock_snapshot"));
    assert!(names.contains(&"devmap_open_dock"));
    assert!(names.contains(&"devmap_start_browser_dock"));
}

#[test]
fn bundled_skill_has_a_narrow_honest_trigger() {
    let skill =
        std::fs::read_to_string(format!("{PLUGIN_ROOT}/skills/live-worktree-dock/SKILL.md"))
            .unwrap();
    let normalized = skill.to_ascii_lowercase();
    assert!(skill.contains("name: live-worktree-dock"));
    assert!(normalized.contains("show, open, or refresh"));
    assert!(skill.contains("devmap_open_dock"));
    assert!(skill.contains("devmap_dock_snapshot"));
    assert!(skill.contains("devmap_start_browser_dock"));
    assert!(skill.contains("placement: right"));
    assert!(normalized.contains("never repeat the authenticated url"));
    assert!(skill.contains("local worktrees"));
    assert!(skill.contains("cross-machine"));
    assert!(!skill.contains("start an HTTP server"));
    assert!(!skill.contains("[TODO:"));
}
