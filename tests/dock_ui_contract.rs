use devmap::dock_asset::{DOCK_MIME_TYPE, DOCK_RESOURCE_URI, dock_html};

#[test]
fn dock_asset_renders_each_lane_through_the_repeated_merge_target() {
    let html = dock_html();
    for contract in [
        "relationship-map",
        "target-left",
        "workspace-node",
        "chat-node",
        "return-edge",
        "target-right",
    ] {
        assert!(
            html.contains(contract),
            "missing graph contract: {contract}"
        );
    }
    assert!(html.contains("Merged into"));
    assert!(html.contains("Not merged"));
    assert!(html.contains("No linked chat"));
    assert!(html.contains("ahead"));
    assert!(html.contains("behind"));
    assert!(!html.contains("<details class=\"group\""));
}

#[test]
fn dock_asset_is_self_contained_and_uses_portable_bridge() {
    let html = dock_html();
    assert_eq!(DOCK_RESOURCE_URI, "ui://devmap/dock/v1.html");
    assert_eq!(DOCK_MIME_TYPE, "text/html;profile=mcp-app");
    assert!(html.contains("ui/initialize"));
    assert!(html.contains("ui/notifications/tool-result"));
    assert!(html.contains("ui/update-model-context"));
    assert!(html.contains("devmap_dock_snapshot"));
    assert!(html.contains("window.parent.postMessage"));
    assert!(!html.contains("https://"));
    assert!(!html.contains("http://"));
    assert!(!html.contains("localStorage"));
    assert!(!html.contains("sessionStorage"));
    assert!(html.len() < 128 * 1024);
}

#[test]
fn dock_asset_is_accessible_responsive_and_explicit_about_uncertainty() {
    let html = dock_html();
    assert_eq!(html.matches("<main").count(), 1);
    assert!(html.contains("type=\"button\""));
    assert!(html.contains("aria-live=\"polite\""));
    assert!(html.contains("Workspaces"));
    assert!(html.contains("Linked chats"));
    assert!(html.contains("Active Agent"));
    assert!(html.contains("UNINSTRUMENTED"));
    assert!(html.contains(".lane.current"));
    assert!(html.contains("Merge unknown"));
    assert!(html.contains("CAPTURE INCOMPLETE"));
    assert!(html.contains("OFFLINE · last update"));
    assert!(html.contains("Date.now() - lastValidAt > 6000"));
    assert!(html.contains("@container"));
    assert!(html.contains("max-width: 519px"));
    assert!(html.contains("prefers-reduced-motion"));
    assert!(html.contains(":focus-visible"));
    assert!(html.contains("visibilitychange"));
}

#[test]
fn dock_asset_validates_untrusted_models_and_never_uses_html_injection() {
    let html = dock_html();
    assert!(html.contains("devmap/dock/1"));
    assert!(html.contains("Number.isSafeInteger"));
    assert!(html.contains("safeRouteId"));
    assert!(html.contains("renderedRevision"));
    assert!(html.contains("value.revision === renderedRevision"));
    assert!(html.contains("2048"));
    assert!(html.contains("textContent"));
    assert!(html.contains("replaceChildren"));
    assert!(!html.contains("innerHTML"));
    for forbidden in ["tool_input", "tool_output", "transcript"] {
        assert!(!html.contains(forbidden), "raw field leaked: {forbidden}");
    }
}

#[test]
fn selection_context_contains_only_portable_route_identity() {
    let html = dock_html();
    assert!(html.contains("DevMap selection: worktree_id="));
    assert!(html.contains("route_id="));
    assert!(!html.contains("DevMap selection: session_id="));
    assert!(!html.contains("DevMap selection: path="));
}

#[test]
fn dock_asset_accepts_exact_codex_task_links_and_renders_the_window_title() {
    let html = dock_html();
    assert!(html.contains("codex_task_cwd"));
    assert!(html.contains("chat.display_title"));
    assert!(html.contains("chat.host_status || chat.status"));
}
