use devmap::dock_asset::{DOCK_MIME_TYPE, DOCK_RESOURCE_URI, dock_html};

#[test]
fn dock_asset_renders_shared_integration_rails_and_fork_stations() {
    let html = dock_html();
    for contract in [
        "relationship-map",
        "integration-rail",
        "fork-station",
        "workspace-branch",
        "task-node",
        "selection-details",
        "Copy hash",
        "No exact tag",
    ] {
        assert!(
            html.contains(contract),
            "missing graph contract: {contract}"
        );
    }
    assert!(html.contains("Merged →"));
    assert!(html.contains("Not merged"));
    assert!(html.contains("Unknown"));
    assert!(html.contains("ahead"));
    assert!(html.contains("behind"));
    assert!(!html.contains("target-left"));
    assert!(!html.contains("target-right"));
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
    assert!(html.contains("Development rail"));
    assert!(html.contains("UNINSTRUMENTED"));
    assert!(html.contains(".workspace-branch.current"));
    assert!(html.contains("Unknown →"));
    assert!(html.contains("CAPTURE INCOMPLETE"));
    assert!(html.contains("OFFLINE · last update"));
    assert!(html.contains("Date.now() - lastValidAt > 6000"));
    assert!(html.contains("@container"));
    assert!(html.contains("max-width: 619px"));
    assert!(html.contains("prefers-reduced-motion"));
    assert!(html.contains(":focus-visible"));
    assert!(html.contains("visibilitychange"));
}

#[test]
fn dock_asset_validates_untrusted_models_and_never_uses_html_injection() {
    let html = dock_html();
    assert!(html.contains("devmap/dock/2"));
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

#[test]
fn dock_refresh_requests_a_fresh_host_task_inventory() {
    let html = dock_html();
    assert!(html.contains("Refresh all"));
    assert!(html.contains("Requesting Codex…"));
    assert!(html.contains("sendFollowUpMessage"));
    assert!(html.contains("method: \"ui/message\""));
    assert!(html.contains("Git refreshed · task names not resynced"));
    assert!(html.contains("Ask Codex: Refresh DevMap"));
    assert!(html.contains(
        "Refresh DevMap task inventory for the current local repository and update the open Dock."
    ));
    assert!(!html.contains("REFRESH_PROMPT = `"));
}
