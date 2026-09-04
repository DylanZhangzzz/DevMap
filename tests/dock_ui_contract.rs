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
fn dock_asset_renders_parallel_branch_rails_with_progressive_density() {
    let html = dock_html();
    for contract in [
        "topology-canvas",
        "branch-rails",
        "branch-rail",
        "fork-node",
        "worktree-stop",
        "agent-summary",
        "density-switch",
        "data-density=\"map\"",
        "aria-pressed",
        "MAP",
        "READ",
        "FULL",
        "integration-rail",
        "task-node",
        "return-state",
        "selection-details",
    ] {
        assert!(
            html.contains(contract),
            "missing Rail View contract: {contract}"
        );
    }
}

#[test]
fn dock_asset_exposes_bounded_branch_disclosure_without_dropping_status_text() {
    let html = dock_html();
    for contract in [
        "collapsed-branches",
        "merged / inactive branches",
        "Merged →",
        "Not merged →",
        "Unknown →",
        "DIRTY",
    ] {
        assert!(
            html.contains(contract),
            "missing branch disclosure contract: {contract}"
        );
    }
}

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
fn dock_asset_matches_the_approved_light_rail_view_theme() {
    let html = dock_html();
    for contract in [
        "color-scheme: light",
        "--bg: #f4f5f7",
        "--surface: #ffffff",
        "--text: #202124",
        "--main-rail: #202124",
        "--accent: #1677ff",
        "--danger: #d92d54",
    ] {
        assert!(
            html.contains(contract),
            "missing light Rail View theme contract: {contract}"
        );
    }
    assert!(!html.contains("color-scheme: dark"));
    assert!(!html.contains("--bg: #090d12"));
}

#[test]
fn dock_asset_places_branch_lanes_on_one_shared_commit_timeline() {
    let html = dock_html();
    for contract in [
        "topology-grid-labels",
        "timeline-station",
        ".timeline-station::after",
        "timeline-head",
        "branch-track",
        "return-edge",
        "stationPercent",
        r#"style.setProperty("--station","#,
        r#"html[data-density="map"] .workspace-short"#,
        r#"html[data-density="full"] .legend"#,
        r#"html[data-density="read"] .branch-rail"#,
        r#"html[data-density="full"] .branch-rail"#,
        ".workspace-branch.current .worktree-stop",
    ] {
        assert!(
            html.contains(contract),
            "missing shared timeline contract: {contract}"
        );
    }
}

#[test]
fn dock_asset_keeps_topology_title_and_density_controls_inside_the_map_shell() {
    let html = dock_html();
    let map_shell = html
        .find("<section class=\"map-frame\"")
        .expect("map shell must exist");
    let title = html
        .find("<h1 id=\"map-title\">Repository topology</h1>")
        .expect("topology title must exist");
    let canvas = html
        .find("<div class=\"relationship-map topology-canvas topology-surface\"")
        .expect("topology canvas must exist");
    assert!(map_shell < title && title < canvas);
    assert!(html.contains("map-toolbar"));
    assert!(html.contains("masthead-actions"));
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

#[test]
fn dock_asset_keeps_worktrees_primary_and_conversations_visible_in_every_density() {
    let html = dock_html();
    for contract in [
        "worktree-identity",
        "conversation-track",
        "conversation-node",
        "conversation-title",
        "agent-identity",
        "conversation-state",
    ] {
        assert!(
            html.contains(contract),
            "missing conversation hierarchy contract: {contract}"
        );
    }
    assert!(!html.contains(r#"html[data-density="map"] .conversation-track { display: none"#));
}

#[test]
fn dock_asset_orders_and_bounds_historical_conversations() {
    let html = dock_html();
    for contract in [
        "function compareConversations",
        "const MAX_RECENT_INACTIVE = 3",
        "historical-conversations",
        "historical conversations",
    ] {
        assert!(
            html.contains(contract),
            "missing bounded conversation history contract: {contract}"
        );
    }
}

#[test]
fn dock_asset_uses_a_sticky_worktree_index_and_scoped_horizontal_viewport() {
    let html = dock_html();
    for contract in [
        "topology-viewport",
        "topology-surface",
        "scrollbar-gutter: stable",
        "function topologyWidth",
        "--topology-width",
    ] {
        assert!(
            html.contains(contract),
            "missing panoramic topology contract: {contract}"
        );
    }
    assert!(html.contains(".topology-viewport"));
    assert!(html.contains("overflow-x: auto"));
    assert!(html.contains(".worktree-identity"));
    assert!(html.contains("position: sticky"));
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
    ] {
        assert!(
            html.contains(contract),
            "missing horizontal pan interaction contract: {contract}"
        );
    }
}

#[test]
fn dock_asset_sticky_worktree_identity_masks_scrolled_content() {
    let html = dock_html();
    assert!(html.contains(".worktree-identity"));
    assert!(html.contains("margin-left: -18px"));
    assert!(html.contains("width: calc(100% + 18px)"));
}

#[test]
fn dock_asset_renders_one_quiet_empty_conversation_state() {
    let html = dock_html();
    assert!(html.contains("summary.hidden = lane.chats.length === 0"));
    assert!(html.contains("No linked conversation"));
}

#[test]
fn dock_asset_preserves_horizontal_panorama_at_narrow_widths() {
    let html = dock_html();
    assert!(!html.contains(".timeline-station { display: none; }"));
    assert!(!html.contains(".rail-line { margin-left: 22px; width: 3px; height: 24px; }"));
    assert!(html.contains("--identity-width: 214px"));
}

#[test]
fn dock_asset_aligns_main_forks_and_branch_edges_in_one_geometry_plane() {
    let html = dock_html();
    for contract in [
        "function alignRailGeometry",
        "--fork-x",
        "--connector-top",
        "--connector-height",
        "transform: translate(-50%, -50%)",
        "requestAnimationFrame(alignAllRailGeometry)",
    ] {
        assert!(
            html.contains(contract),
            "missing shared rail geometry contract: {contract}"
        );
    }
}
