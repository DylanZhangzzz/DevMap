use devmap::dock_asset::{DOCK_MIME_TYPE, DOCK_RESOURCE_URI, dock_html};

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
    assert!(html.contains("Current"));
    assert!(html.contains("Active"));
    assert!(html.contains("Stale or uninstrumented"));
    assert!(html.contains("<details class=\"group\""));
    assert!(!html.contains("<details class=\"group\" open"));
    assert!(html.contains(".group-current .row"));
    assert!(html.contains("Status"));
    assert!(html.contains("Confidence"));
    assert!(html.contains("CAPTURE INCOMPLETE"));
    assert!(html.contains("OFFLINE · last update"));
    assert!(html.contains("Date.now() - lastValidAt > 6000"));
    assert!(html.contains("@container"));
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
