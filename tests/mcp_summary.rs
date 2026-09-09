mod support;

use devmap::mcp::McpRuntime;
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

fn open_runtime(root: &Path) -> McpRuntime {
    let mut runtime = McpRuntime::open(root).unwrap();
    runtime.handle(&json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"summary-test","version":"1"}
    }})).unwrap();
    runtime
}
fn call(runtime: &mut McpRuntime, arguments: Value) -> Value {
    runtime
        .handle(
            &json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{
                "name":"devmap_read_map","arguments":arguments
            }}),
        )
        .unwrap()
}
fn success(response: &Value) -> &Value {
    assert!(response.get("error").is_none(), "{response}");
    assert_ne!(response["result"]["isError"], true, "{response}");
    &response["result"]["structuredContent"]
}
fn summary(response: &Value) -> &Value {
    let value = success(response);
    assert_eq!(value["schema_version"], "devmap-summary/1");
    assert!(serde_json::to_vec(&response["result"]).unwrap().len() <= 32768);
    assert!(value["snapshot_id"].as_str().is_some_and(|s| !s.is_empty()));
    value
}
fn files(root: &Path) -> BTreeMap<std::path::PathBuf, Vec<u8>> {
    fn walk(base: &Path, path: &Path, out: &mut BTreeMap<std::path::PathBuf, Vec<u8>>) {
        if !path.exists() {
            return;
        }
        for entry in std::fs::read_dir(path).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(base, &path, out);
            } else {
                out.insert(
                    path.strip_prefix(base).unwrap().to_path_buf(),
                    std::fs::read(path).unwrap(),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

#[test]
fn explicit_summary_is_bounded_read_only_and_keeps_original_observation_coverage() {
    let repo = support::committed_repo();
    let mut runtime = open_runtime(repo.path());
    let response = call(&mut runtime, json!({"view":"summary"}));
    let model = summary(&response);
    let legacy = call(&mut runtime, json!({}));
    assert_eq!(success(&legacy)["schema_version"], "devmap/dock/4");
    assert_eq!(
        model["repository_id"],
        legacy["result"]["structuredContent"]["repository_id"]
    );
    assert_eq!(
        model["current_worktree_id"],
        legacy["result"]["structuredContent"]["current_worktree_id"]
    );
    assert_eq!(model["source_truncated"], false);
    assert_eq!(
        model["counts"],
        legacy["result"]["structuredContent"]["counts"]
    );
    assert_eq!(
        model["pages"]["workspaces"]["items"][0]["git_status"]["merged"],
        Value::Null
    );
    assert_eq!(model["execution"]["merge_ready"], false);
    let observations = &model["observations"];
    assert!(observations["git_cycle"].as_u64().is_some());
    time::OffsetDateTime::parse(
        observations["git_observed_at"].as_str().unwrap(),
        &time::format_description::well_known::Rfc3339,
    )
    .unwrap();
    assert!(
        observations
            .as_object()
            .unwrap()
            .contains_key("store_inputs_observed_at")
    );
    assert_eq!(observations["store_generation"], Value::Null);
    assert_eq!(
        observations["task_observation"],
        legacy["result"]["structuredContent"]["task_observation"]
    );
    assert_eq!(observations["task_observation"]["complete"], false);
    assert!(
        !repo.path().join(".git/devmap").exists(),
        "summary/read created storage"
    );
}

#[test]
fn summary_rejects_inventory_without_creating_storage() {
    let repo = support::committed_repo();
    let mut runtime = open_runtime(repo.path());
    for arguments in [
        json!({"view":"summary","codex_tasks":[],"codex_tasks_complete":true}),
        json!({"view":"summary","entity_id":"some-entity"}),
    ] {
        let response = call(&mut runtime, arguments);
        assert_eq!(response["result"]["isError"], true, "{response}");
        assert!(serde_json::to_vec(&response["result"]).unwrap().len() <= 32768);
    }
    assert!(!repo.path().join(".git/devmap").exists());
}

#[test]
fn task_and_warning_pages_retain_one_snapshot_and_enumerate_every_item() {
    let repo = support::committed_repo();
    let presence = repo.path().join(".git/devmap/presence/v1");
    std::fs::create_dir_all(&presence).unwrap();
    for index in 0..20 {
        std::fs::write(
            presence.join(format!("broken-{index:02}.json")),
            b"not valid JSON",
        )
        .unwrap();
    }
    let mut runtime = open_runtime(repo.path());
    let tasks = (0..24)
        .map(|index| {
            json!({
                "id":format!("00000000-0000-4000-8000-{index:012}"),
                "title":format!("task-{index} {}", "汉字🧭\"\\".repeat(50)),
                "status":"idle","lifecycle":"present","cwd":repo.path(),
                "updatedAt":1_789_000_000u64,"hostId":"local","kind":"codex"
            })
        })
        .collect::<Vec<_>>();
    let map = call(
        &mut runtime,
        json!({"codex_tasks":tasks,"codex_tasks_complete":true}),
    );
    let model = success(&map);
    let expected_tasks = model["lanes"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|lane| lane["chats"].as_array().unwrap())
        .filter_map(|chat| chat["codex_thread_id"].as_str().map(str::to_owned))
        .map(|id| (id, 1usize))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(expected_tasks.len(), 24);
    let mut expected_warnings = BTreeMap::new();
    for warning in model["warnings"].as_array().unwrap() {
        *expected_warnings
            .entry(warning.to_string())
            .or_insert(0usize) += 1;
    }
    assert!(expected_warnings.values().sum::<usize>() >= 20);
    let before = files(&repo.path().join(".git/devmap"));
    let first = call(&mut runtime, json!({"view":"summary"}));
    let first = summary(&first).clone();
    assert_eq!(files(&repo.path().join(".git/devmap")), before);
    assert_eq!(first["source_truncated"], false);
    assert_eq!(first["counts"], model["counts"]);
    assert_eq!(first["pages"]["tasks"]["incomplete"], true);
    assert_eq!(
        first["observations"]["task_observation"],
        model["task_observation"]
    );
    let snapshot = first["snapshot_id"].clone();
    let observations = first["observations"].clone();
    let original_cursor = first["pages"]["tasks"]["next_cursor"]
        .as_str()
        .unwrap()
        .to_owned();
    let mut other = open_runtime(repo.path());
    let rejected = call(
        &mut other,
        json!({"view":"summary","cursor":original_cursor}),
    );
    assert_eq!(
        rejected["result"]["isError"], true,
        "cross-client cursor was accepted"
    );
    // A normal live map refresh must not splice its replacement inventory into old pages.
    success(&call(
        &mut runtime,
        json!({"codex_tasks":[],"codex_tasks_complete":true}),
    ));
    let after_inventory = files(&repo.path().join(".git/devmap"));
    for (collection, expected) in [("tasks", expected_tasks), ("warnings", expected_warnings)] {
        let mut page = first["pages"][collection].clone();
        let mut seen = BTreeMap::new();
        let mut tokens = BTreeSet::new();
        for _ in 0..32 {
            assert_eq!(
                page["total"].as_u64().unwrap() as usize,
                expected.values().sum::<usize>()
            );
            let items = page["items"].as_array().unwrap();
            assert_eq!(page["included"].as_u64().unwrap() as usize, items.len());
            assert!(!items.is_empty() && items.len() <= 8);
            for item in items {
                let key = if collection == "tasks" {
                    item["id"].as_str().unwrap().to_owned()
                } else {
                    item.to_string()
                };
                *seen.entry(key).or_insert(0usize) += 1;
            }
            let Some(cursor) = page["next_cursor"].as_str() else {
                assert_eq!(page["incomplete"], false);
                break;
            };
            assert_eq!(page["incomplete"], true);
            assert!(tokens.insert(cursor.to_owned()), "pagination loop");
            let response = call(&mut runtime, json!({"view":"summary","cursor":cursor}));
            let next = summary(&response);
            assert_eq!(next["snapshot_id"], snapshot);
            assert_eq!(next["observations"], observations);
            let retry = call(&mut runtime, json!({"view":"summary","cursor":cursor}));
            assert_eq!(response["result"], retry["result"]);
            page = next["pages"][collection].clone();
        }
        assert_eq!(seen, expected);
    }
    assert_eq!(files(&repo.path().join(".git/devmap")), after_inventory);
    let replacement = call(&mut runtime, json!({"view":"summary"}));
    assert_ne!(summary(&replacement)["snapshot_id"], snapshot);
    let expired = call(
        &mut runtime,
        json!({"view":"summary","cursor":original_cursor}),
    );
    assert_eq!(expired["result"]["isError"], true);
}
