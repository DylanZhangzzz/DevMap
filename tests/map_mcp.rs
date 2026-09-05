mod support;
use devmap::mcp::McpRuntime;
use serde_json::{Value, json};

fn initialized_runtime(path: &std::path::Path) -> McpRuntime {
    let mut runtime = McpRuntime::open(path).unwrap();
    runtime.handle(&json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"map-test","version":"1"}}})).unwrap();
    runtime
}

fn call(runtime: &mut McpRuntime, name: &str, arguments: Value) -> Value {
    runtime.handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":name,"arguments":arguments}})).unwrap()
}

#[test]
fn passenger_lifecycle_is_exposed_to_agents_and_controls_unattended_work() {
    let repo = support::committed_repo();
    std::fs::write(repo.path().join("unfinished.txt"), "work").unwrap();
    let mut runtime = initialized_runtime(repo.path());
    let mut task = json!({"id":"01a00000-0000-7000-8000-000000000001","title":"Paused chat","status":"idle","lifecycle":"present","cwd":repo.path(),"updatedAt":1,"hostId":"local","kind":"codex"});
    let implicit = call(
        &mut runtime,
        "devmap_read_map",
        json!({"view":"agent","codex_tasks":[]}),
    );
    assert_eq!(
        implicit["result"]["structuredContent"]["workspace_facts"]["passengers"]["state"],
        "unknown"
    );
    for status in ["idle", "completed", "waiting", "notLoaded"] {
        task["status"] = json!(status);
        let result = call(
            &mut runtime,
            "devmap_read_map",
            json!({"view":"agent","codex_tasks":[task],"codex_tasks_complete":true}),
        );
        assert_eq!(
            result["result"]["structuredContent"]["workspace_facts"]["passengers"]["state"],
            "occupied",
            "{result}"
        );
    }
    for (lifecycle, state, risk) in [
        ("present", "occupied", false),
        ("archived", "unattended", true),
        ("deleted", "unattended", true),
        ("unknown", "unknown", false),
    ] {
        task["lifecycle"] = json!(lifecycle);
        let result = call(
            &mut runtime,
            "devmap_read_map",
            json!({"view":"agent","codex_tasks":[task],"codex_tasks_complete":true}),
        );
        assert_ne!(result["result"]["isError"], true, "{result}");
        let summary = &result["result"]["structuredContent"]["workspace_facts"]["passengers"];
        assert_eq!(
            result["result"]["structuredContent"]["task_observation"]["scope"],
            "unarchived_chats"
        );
        assert_eq!(summary["state"], state, "{result}");
        assert_eq!(summary["unattended_work"], risk, "{result}");
    }
    let result = call(
        &mut runtime,
        "devmap_read_map",
        json!({"view":"agent","codex_tasks":[],"codex_tasks_complete":false}),
    );
    assert_eq!(
        result["result"]["structuredContent"]["workspace_facts"]["passengers"]["state"],
        "unknown"
    );
    let result = call(
        &mut runtime,
        "devmap_read_map",
        json!({"view":"agent","codex_tasks":[],"codex_tasks_complete":true}),
    );
    assert_eq!(
        result["result"]["structuredContent"]["workspace_facts"]["passengers"]["state"],
        "unattended"
    );
    task.as_object_mut().unwrap().remove("lifecycle");
    let result = call(
        &mut runtime,
        "devmap_read_map",
        json!({"view":"agent","codex_tasks":[task],"codex_tasks_complete":true}),
    );
    assert_eq!(
        result["result"]["structuredContent"]["workspace_facts"]["passengers"]["state"],
        "unknown"
    );
}

#[test]
fn agent_context_keeps_delivery_intent_separate_from_observed_facts() {
    let repo = support::committed_repo();
    let before = support::source_snapshot(repo.path());
    let mut runtime = initialized_runtime(repo.path());
    let map = call(&mut runtime, "devmap_read_map", json!({}));
    let wt = &map["result"]["structuredContent"]["current_worktree_id"];
    let args = json!({"request_id":"delivery", "expected_revision":0,"worktree_id":wt,"goal":"Login","source":"User plan","target_ref":"refs/heads/main",
        "delivery":{"mode":"auto_merge","conditions":["Login tests pass"],"authorization_source":"User: merge into main after tests pass"}});
    let saved = call(&mut runtime, "devmap_set_route_plan", args.clone());
    assert_ne!(saved["result"]["isError"], true, "{saved}");
    let mut runtime = initialized_runtime(repo.path());
    let context = call(&mut runtime, "devmap_read_map", json!({"view":"agent"}));
    let context = &context["result"]["structuredContent"];
    assert_eq!(&context["workspace"]["worktree_id"], wt);
    assert_eq!(context["route_plans"][0]["delivery"]["mode"], "auto_merge");
    assert_eq!(context["execution"]["checks_status"], "unverified");
    assert_eq!(context["execution"]["merge_ready"], false);
    assert_eq!(context["execution"]["authorization_verified"], false);
    let mut invalid = args;
    invalid["request_id"] = json!("invalid-delivery");
    invalid["target_ref"] = Value::Null;
    assert_eq!(
        call(&mut runtime, "devmap_set_route_plan", invalid)["result"]["isError"],
        true
    );
    assert_eq!(support::source_snapshot(repo.path()), before);
}

#[test]
fn legacy_plans_default_to_manual_and_agent_selection_fails_closed() {
    let repo = support::committed_repo();
    let mut runtime = initialized_runtime(repo.path());
    let map = call(&mut runtime, "devmap_read_map", json!({}));
    let saved = call(
        &mut runtime,
        "devmap_set_route_plan",
        json!({"request_id":"manual", "expected_revision":0,"worktree_id":map["result"]["structuredContent"]["current_worktree_id"],"goal":"Fix","source":"Plan"}),
    );
    assert_eq!(
        saved["result"]["structuredContent"]["delivery"]["mode"],
        "manual"
    );
    let result = call(
        &mut runtime,
        "devmap_read_map",
        json!({"view":"agent","entity_id":"missing"}),
    );
    assert_eq!(result["result"]["isError"], true);
}

#[test]
fn discover_three_map_tools_and_preserve_capture_tools() {
    let repo = support::committed_repo();
    let mut runtime = initialized_runtime(repo.path());
    let listed = runtime
        .handle(&json!({"jsonrpc":"2.0","id":1,"method":"tools/list"}))
        .unwrap();
    let names: Vec<_> = listed["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(
        names,
        [
            "devmap_open_map",
            "devmap_read_map",
            "devmap_set_route_plan",
            "devmap_record_requirement",
            "devmap_record_decision",
            "devmap_record_evidence"
        ]
    );
    let descriptor = &listed["result"]["tools"][2];
    assert_eq!(descriptor["annotations"]["readOnlyHint"], false);
}

#[test]
fn set_read_and_reopen_plan_without_git_mutation() {
    let repo = support::committed_repo();
    let before = support::source_snapshot(repo.path());
    let mut runtime = initialized_runtime(repo.path());
    let snapshot = call(&mut runtime, "devmap_read_map", json!({}));
    let worktree_id = snapshot["result"]["structuredContent"]["current_worktree_id"]
        .as_str()
        .unwrap();
    let result = call(
        &mut runtime,
        "devmap_set_route_plan",
        json!({
            "request_id":"req-1", "expected_revision":0, "worktree_id":worktree_id,
            "goal":"Login fix", "target_ref":"refs/heads/main", "milestones":["Verify"], "source":"Explicit user plan"
        }),
    );
    assert!(result.get("error").is_none(), "{result}");
    assert_ne!(result["result"]["isError"], true, "{result}");
    let route_id = result["result"]["structuredContent"]["route_id"]
        .as_str()
        .unwrap();
    let mut reopened = initialized_runtime(repo.path());
    let details = call(
        &mut reopened,
        "devmap_read_map",
        json!({"entity_id":route_id}),
    );
    assert_eq!(
        details["result"]["structuredContent"]["entity"]["goal"],
        "Login fix"
    );
    let map = call(&mut reopened, "devmap_open_map", json!({}));
    assert!(map["result"]["_meta"]["ui"]["resourceUri"].is_string());
    assert_eq!(
        map["result"]["structuredContent"]["route_plans"]
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert_eq!(support::source_snapshot(repo.path()), before);
}

#[test]
fn legacy_read_alias_and_invalid_new_arguments() {
    let repo = support::committed_repo();
    let mut runtime = initialized_runtime(repo.path());
    assert!(call(&mut runtime, "devmap_dock_snapshot", json!({}))["result"]["structuredContent"]["revision"].is_number());
    assert_eq!(
        call(&mut runtime, "devmap_open_map", json!({"surface":"shell"}))["result"]["isError"],
        true
    );
    assert_eq!(
        call(
            &mut runtime,
            "devmap_read_map",
            json!({"entity_id":"missing"})
        )["result"]["isError"],
        true
    );
}

#[test]
fn conflicting_plan_write_returns_latest_plan_for_reconciliation() {
    let repo = support::committed_repo();
    let mut runtime = initialized_runtime(repo.path());
    let model = call(&mut runtime, "devmap_read_map", json!({}));
    let mut args = json!({"request_id":"one","expected_revision":0,"worktree_id":model["result"]["structuredContent"]["current_worktree_id"],"goal":"Goal","source":"User instruction"});
    let first = call(&mut runtime, "devmap_set_route_plan", args.clone());
    let plan = &first["result"]["structuredContent"];
    args["route_id"] = plan["route_id"].clone();
    args["request_id"] = json!("two");
    let conflict = call(&mut runtime, "devmap_set_route_plan", args);
    assert_eq!(conflict["result"]["isError"], true);
    assert_eq!(
        &conflict["result"]["structuredContent"]["current_plan"],
        plan
    );
}

#[test]
fn human_history_change_is_observed_without_restoring_it() {
    let repo = support::committed_repo();
    support::git(
        repo.path(),
        ["commit", "--allow-empty", "-m", "Second point"],
    );
    let mut runtime = initialized_runtime(repo.path());
    call(&mut runtime, "devmap_read_map", json!({}));
    support::git(repo.path(), ["reset", "--soft", "HEAD~1"]);
    let before = support::source_snapshot(repo.path());
    let map = call(&mut runtime, "devmap_read_map", json!({}));
    assert!(
        map["result"]["structuredContent"]["warnings"]
            .as_array()
            .unwrap()
            .iter()
            .any(|w| w["code"] == "workspace_history_changed")
    );
    assert_eq!(support::source_snapshot(repo.path()), before);
}
