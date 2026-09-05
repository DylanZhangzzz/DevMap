use devmap::dock_asset::{DOCK_MIME_TYPE, DOCK_RESOURCE_URI, dock_html};

// Semantic/runtime renderer coverage lives in tests/dock_renderer.cjs.
// Browser geometry, responsive pixels and visual acceptance remain separate gates.

#[test]
fn dock_asset_uses_neutral_surfaces_and_small_area_semantic_colors() {
    let html = dock_html();
    for token in [
        "--bg-canvas",
        "--surface-raised",
        "--accent",
        "--success",
        "--warning",
        "--danger",
    ] {
        assert!(html.contains(token), "missing color-system token: {token}");
    }
    assert!(!html.contains("--branch-soft"));
    assert!(!html.contains("--dev-soft"));
}

#[test]
fn dock_asset_is_self_contained_and_uses_portable_bridge() {
    let html = dock_html();
    assert_eq!(DOCK_RESOURCE_URI, "ui://devmap/dock/v1.html");
    assert_eq!(DOCK_MIME_TYPE, "text/html;profile=mcp-app");
    assert!(html.contains("ui/initialize"));
    assert!(html.contains("ui/notifications/tool-result"));
    assert!(html.contains("ui/update-model-context"));
    assert!(html.contains("devmap_read_map"));
    assert!(html.contains("window.parent.postMessage"));
    assert!(!html.contains("https://"));
    // SVG namespace identifies elements; it is not an external request.
    assert!(
        !html
            .replace("http://www.w3.org/2000/svg", "")
            .contains("http://")
    );
    assert!(!html.contains("localStorage"));
    assert!(!html.contains("sessionStorage"));
    // Includes persistent journey navigation; remains below the 512 KiB transport cap.
    assert!(html.len() < 148 * 1024);
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
fn dock_task_navigation_uses_only_a_validated_codex_thread_id() {
    let html = dock_html();
    for contract in [
        "safeCodexThreadId",
        "codex_thread_id",
        "requestTaskNavigation",
        "Open the local Codex task with id",
        "Open this task from Codex",
        "sendFollowUpMessage",
        "method: \"ui/message\"",
        "codex://threads/",
        "verify the destination",
        "Copy task ID",
    ] {
        assert!(
            html.contains(contract),
            "missing navigation contract: {contract}"
        );
    }
    assert!(!html.contains("Open the local Codex task with title"));
    assert!(!html.contains("codex://threads/${"));
}

#[test]
fn dock_task_navigation_does_not_turn_presence_only_records_into_links() {
    let html = dock_html();
    assert!(html.contains("if (!chat.codex_thread_id || chat.lifecycle === \"deleted\")"));
    assert!(html.contains("node.dataset.navigable"));
    assert!(html.contains("agent-task-node ${category} ${stateValue}`"));
    assert!(html.contains("graphButton("));
}

#[test]
fn dock_task_navigation_recovers_when_portable_bridge_does_not_reply() {
    let html = dock_html();
    for contract in [
        "NAVIGATION_RESPONSE_TIMEOUT_MS",
        "pendingNavigationTimeouts",
        "function finishTaskNavigationRequest",
        "setTimeout(() => finishTaskNavigationRequest(id, \"Codex task could not be opened\")",
        "pendingRequests.delete(id)",
        "pendingNavigationNodes.delete(id)",
        "clearTimeout(timeout)",
        "let navigationRequestId = null",
        "if (navigationRequestId !== null) finishTaskNavigationRequest",
        "finishTaskNavigationRequest(message.id, message.error ? \"Codex task could not be opened\" : \"Codex accepted the task request · verify the destination\")",
        "if (pendingRequests.has(message.id))",
    ] {
        assert!(
            html.contains(contract),
            "missing navigation recovery contract: {contract}"
        );
    }
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

#[test]
fn dock_asset_supports_safe_horizontal_pan_inputs() {
    let html = dock_html();
    for contract in [
        "function installPanControls",
        "pointerdown",
        "setPointerCapture",
        "event.shiftKey",
        r#"event.target.closest("button, a, input, textarea, select, [data-no-pan]")"#,
        "ArrowLeft",
        "ArrowRight",
        "Pan repository topology",
        "id=\"pan-left\"",
        "id=\"pan-right\"",
    ] {
        assert!(
            html.contains(contract),
            "missing horizontal pan interaction contract: {contract}"
        );
    }
}

#[test]
fn dock_asset_preserves_required_titles_and_hides_unselected_inspector() {
    let html = dock_html();
    assert!(html.contains("<title>DevMap · Rail View — Repository topology</title>"));
    assert!(html.contains(">DevMap · Rail View</p>"));
    assert!(html.contains("<h1 id=\"map-title\">Repository topology</h1>"));
    assert!(html.contains("id=\"selection-details\" aria-labelledby=\"selection-title\" hidden"));
    assert!(html.contains("id=\"interaction-feedback\" role=\"status\" aria-live=\"polite\""));
    assert!(html.contains("id=\"task-inventory\" aria-live=\"polite\""));
    assert_eq!(html.matches("<main").count(), 1);
}

#[test]
fn dock_asset_rejects_html_injection_and_keeps_embedded_validation_bounds() {
    let html = dock_html();
    assert!(html.contains("Core.validateSnapshot(value)"));
    assert!(html.contains("devmap/dock/3"));
    assert!(html.contains("devmap/dock/4"));
    assert!(html.contains("Number.isSafeInteger"));
    assert!(html.contains("safeRouteId"));
    assert!(html.contains("textContent"));
    assert!(html.contains("replaceChildren"));
    assert!(!html.contains("innerHTML"));
    for forbidden in ["tool_input", "tool_output", "transcript"] {
        assert!(!html.contains(forbidden), "raw field leaked: {forbidden}");
    }
}

#[test]
fn dock_asset_keeps_accessible_scrolling_and_transport_age_state() {
    let html = dock_html();
    assert!(html.contains("tabindex=\"0\" aria-label=\"Scrollable repository topology\""));
    assert!(html.contains("Locate current workspace"));
    assert!(html.contains("prefers-reduced-motion"));
    assert!(html.contains(":focus-visible"));
    assert!(html.contains("visibilitychange"));
    assert!(html.contains("GIT OFFLINE · last observation"));
    assert!(html.contains("Date.now() - lastValidAt > 6000"));
    assert!(html.contains("CAPTURE INCOMPLETE"));
}

#[test]
fn dock_asset_tracks_exploration_state_independently_from_rendered_nodes() {
    let html = dock_html();
    for contract in [
        "selectedWorkspaceId",
        "selectedTaskId",
        "expandedWorkspaces",
        "expandedConversationHistory",
        "viewportPosition",
        "reconcileExplorationState",
        "Selected task ",
        "Selected workspace ",
        "is no longer available",
    ] {
        assert!(
            html.contains(contract),
            "missing exploration state contract: {contract}"
        );
    }
}
