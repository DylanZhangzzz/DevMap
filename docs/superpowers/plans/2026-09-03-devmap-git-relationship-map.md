# DevMap Git Relationship Map Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace the worktree card list with a single responsive Git relationship map that shows each workspace, its exact associated chat or Agent, its branch state relative to the selected development target, and whether it has merged back, while adding a supported one-command right-Browser reopen path.

**Architecture:** A new read-only Git relationship resolver computes one repository target plus per-worktree dirty, ahead, behind, and exact ancestry facts. `DockReadModel` joins those facts with existing exact Presence associations; the shared self-contained frontend renders them as left-to-right lanes ending at a repeated merge target. The MCP runtime owns at most one on-demand loopback Viewer and exposes its authenticated URL only through structured tool output so the Codex skill can open or reopen it in the right Browser pane.

**Tech Stack:** Rust 2024 (Rust 1.96), `std::process::Command`, Serde/JSON, Tiny HTTP with loopback-only SSE, MCP over STDIO, self-contained HTML/CSS/SVG/JavaScript, Cargo integration tests, Codex personal plugins.

**Spec:** `docs/superpowers/specs/2026-09-02-devmap-live-worktree-dock-design.md`

## Global Constraints

- The MCP snapshot and embedded-App paths open no TCP listener; only `devmap_start_browser_dock` or explicit `devmap view --live` may bind to `127.0.0.1` on a random port.
- Git inspection is read-only and uses exact ancestry; branch names, a clean tree, or chat state never imply that a branch merged.
- Target selection order is `devmap.developmentTarget`, local `dev`, local `develop`, remote default branch, local `main`, then local `master`; an unresolved target produces an explicit unknown relationship.
- A chat attaches only through an exact session/worktree identity already present in DevMap capture data; missing association stays `No linked chat`, and `Ignored` requires an explicit ignore fact.
- Presence and Dock payloads contain no prompts, commands, patches, tool arguments/results, file contents, or transcript text.
- Frontend assets stay below 128 KiB, use no external URLs or browser storage, perform no HTML injection, and keep the document free of horizontal overflow at 320, 360, 520, 736, and 1,024 px.
- The right-side target is rendered for every lane; green reaches it only for merged branches, while amber stops before it and carries exact ahead/behind values for unmerged branches.
- Closing the Browser tab does not mutate DevMap; a repeated start call reuses the healthy Viewer, and dropping the MCP runtime stops it.
- Preserve the existing `devmap/dock/1` schema identifier and add bounded fields compatibly; old consumers may ignore the additional keys.

---

### Task 1: Exact Git Relationship Resolver

**Files:**
- Create: `src/git_relationship.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Test: `tests/git_relationship.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**
- Consumes: `SourceWorkspace`, `WorktreeDescriptor`, and read-only Git commands executed inside each `WorktreeDescriptor::root`.
- Produces: `DevelopmentTarget { name: String, ref_name: String, source: TargetSource }`, `GitRelationship { base_target: Option<String>, merge_target: Option<String>, merged: Option<bool>, ahead: Option<u32>, behind: Option<u32>, dirty: bool, changed_file_count: u32 }`, and `GitRelationshipResolver::resolve(&SourceWorkspace, &[WorktreeDescriptor]) -> Result<GitRelationshipReport, DevMapError>`.

- [ ] **Step 1: Write target-selection tests that name the precedence breaks**

Create disposable repositories with competing refs and assert literal outcomes:

```rust
#[test]
fn configured_target_wins_over_dev_and_remote_default() {
    let repo = committed_repo();
    git(repo.path(), ["branch", "dev"]);
    git(repo.path(), ["branch", "release"]);
    git(repo.path(), ["config", "devmap.developmentTarget", "release"]);
    let workspace = SourceGitInspector::open(repo.path()).unwrap().workspace().unwrap();
    let report = GitRelationshipResolver::resolve(&workspace, &WorktreeScanner::scan(&workspace).unwrap()).unwrap();
    assert_eq!(report.target.unwrap().name, "release");
}

#[test]
fn local_dev_wins_over_develop_and_main() {
    let repo = committed_repo();
    git(repo.path(), ["branch", "develop"]);
    git(repo.path(), ["branch", "dev"]);
    let workspace = SourceGitInspector::open(repo.path()).unwrap().workspace().unwrap();
    let report = GitRelationshipResolver::resolve(&workspace, &WorktreeScanner::scan(&workspace).unwrap()).unwrap();
    assert_eq!(report.target.unwrap().name, "dev");
}
```

- [ ] **Step 2: Run the target tests and observe the missing-module failure**

Run: `cargo test --test git_relationship`

Expected: FAIL because `devmap::git_relationship` does not exist.

- [ ] **Step 3: Add the resolver types and deterministic target selection**

Implement these public types and keep the command helper private:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetSource { Config, LocalDev, LocalDevelop, RemoteDefault, LocalMain, LocalMaster }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DevelopmentTarget { pub name: String, pub ref_name: String, pub source: TargetSource }

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct GitRelationship {
    pub base_target: Option<String>,
    pub merge_target: Option<String>,
    pub merged: Option<bool>,
    pub ahead: Option<u32>,
    pub behind: Option<u32>,
    pub dirty: bool,
    pub changed_file_count: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitRelationshipReport {
    pub target: Option<DevelopmentTarget>,
    pub by_worktree_id: BTreeMap<String, GitRelationship>,
    pub warnings: Vec<GitRelationshipWarning>,
}
```

Probe refs with `git show-ref --verify --quiet`, resolve `refs/remotes/<remote>/HEAD` through `git symbolic-ref --quiet`, reject configured values outside the bounded ref grammar, and return `target: None` rather than guessing when no candidate resolves.

- [ ] **Step 4: Write ancestry, divergence, and dirty-count tests**

```rust
#[test]
fn relationship_distinguishes_merged_unmerged_and_dirty_worktrees() {
    let repo = committed_repo();
    let merged = linked_worktree(repo.path(), "codex/merged");
    std::fs::write(merged.path().join("merged.txt"), "merged\n").unwrap();
    git(merged.path(), ["add", "merged.txt"]);
    git(merged.path(), ["commit", "-m", "merged work"]);
    git(repo.path(), ["merge", "--ff-only", "codex/merged"]);

    let open = linked_worktree(repo.path(), "codex/open");
    std::fs::write(open.path().join("open.txt"), "open\n").unwrap();
    git(open.path(), ["add", "open.txt"]);
    git(open.path(), ["commit", "-m", "open work"]);
    std::fs::write(open.path().join("dirty.txt"), "dirty\n").unwrap();

    let workspace = SourceGitInspector::open(repo.path()).unwrap().workspace().unwrap();
    let worktrees = WorktreeScanner::scan(&workspace).unwrap();
    let report = GitRelationshipResolver::resolve(&workspace, &worktrees).unwrap();
    let merged_row = row_for_branch(&report, &worktrees, "codex/merged");
    let open_row = row_for_branch(&report, &worktrees, "codex/open");
    assert_eq!((merged_row.merged, merged_row.ahead, merged_row.behind), (Some(true), Some(0), Some(0)));
    assert_eq!((open_row.merged, open_row.ahead, open_row.behind), (Some(false), Some(1), Some(0)));
    assert_eq!((open_row.dirty, open_row.changed_file_count), (true, 1));
}
```

- [ ] **Step 5: Run the relationship test and observe a behavioral failure**

Run: `cargo test --test git_relationship relationship_distinguishes_merged_unmerged_and_dirty_worktrees -- --exact`

Expected: FAIL because per-worktree facts are not yet projected.

- [ ] **Step 6: Implement exact per-worktree facts**

For every descriptor, parse `git rev-list --left-right --count <target-ref>...<head>` as `behind ahead`; `ahead == 0` is the exact ancestry proof that the worktree HEAD is reachable from the target. Cap both counts at `u32::MAX`, parse `git status --porcelain=v1 -z --untracked-files=normal` while counting rename/copy pairs once, deduplicate identical root/HEAD queries, and run independent roots through a bounded worker set. Convert a per-row command failure into an unknown row plus a bounded warning; do not discard the worktree.

- [ ] **Step 7: Run resolver tests and the existing worktree suite**

Run: `cargo test --test git_relationship --test worktrees`

Expected: PASS.

- [ ] **Step 8: Commit the resolver slice after TG0 message confirmation**

Stage: `git add src/git_relationship.rs src/lib.rs src/error.rs tests/git_relationship.rs tests/support/mod.rs`

Proposed message: `[FEAT](dock): Resolve exact Git merge relationships`

### Task 2: Join Relationships and Exact Chat Associations into the Dock Model

**Files:**
- Modify: `src/dock.rs`
- Modify: `tests/dock_model.rs`
- Modify: `tests/live_dock_acceptance.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**
- Consumes: `GitRelationshipResolver::resolve`, existing `PresenceRecord` values keyed by exact `worktree_id`, and optional explicit session data already carried by Presence.
- Produces: `DockChat`, `DockLane`, `DockReadModel::development_target`, and a flat `DockReadModel::lanes` sequence ordered current-first then by workspace path.

- [ ] **Step 1: Write a reducer test for the full user-visible lane**

```rust
#[test]
fn reducer_projects_workspace_chat_branch_and_merge_target_in_one_lane() {
    let fixture = dock_reducer_fixture();
    let model = DockReducer::new(NoRoutes).reduce(
        &fixture.workspace,
        fixture.worktrees,
        fixture.presence,
        fixture.journals,
        fixture.now,
    ).unwrap();
    let lane = model.lanes.iter().find(|lane| !lane.chats.is_empty()).unwrap();
    assert_eq!(lane.chats[0].session_id, "active-session");
    assert_eq!(lane.chats[0].association_source, "presence_worktree_id");
    assert_eq!(lane.relationship.merge_target.as_deref(), Some("main"));
    assert_eq!(lane.relationship.merged, Some(false));
}
```

- [ ] **Step 2: Run the focused reducer test and observe missing fields**

Run: `cargo test --test dock_model reducer_projects_workspace_chat_branch_and_merge_target_in_one_lane -- --exact`

Expected: FAIL because `lanes`, `chat`, and `relationship` do not exist.

- [ ] **Step 3: Add bounded model types and perform the join**

Use these serialized structures:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct DockChat {
    pub session_id: String,
    pub actor_id: String,
    pub host: String,
    pub status: PresenceStatus,
    pub confidence: Confidence,
    pub association_source: &'static str,
}

pub struct DockLane {
    pub workspace_path: String,
    pub chats: Vec<DockChat>,
    pub relationship: GitRelationship,
}

pub struct DockReadModel {
    // retain schema, revision, warnings, truncation, and compatibility groups
    pub development_target: Option<DevelopmentTarget>,
    pub lanes: Vec<DockEntry>,
}
```

Construct `chats` only from `PresenceRecord` values already matched by repository and exact `worktree_id`. Use the bounded `actor_id` as the visible label and the session ID as identity; do not derive a title from filesystem paths or task recency. Preserve `current`, `active`, and `stale_or_uninstrumented` during this schema version for existing clients, but make `lanes` the frontend source. During size truncation, retain every affordable workspace lane first and then retain as many attached chats as fit, so a large chat collection cannot erase the workspace graph.

- [ ] **Step 4: Add a no-guessing test for an uninstrumented worktree**

```rust
#[test]
fn reducer_does_not_attach_a_chat_without_exact_presence() {
    let mut fixture = dock_reducer_fixture();
    fixture.presence.records.clear();
    let model = DockReducer::new(NoRoutes).reduce(
        &fixture.workspace, fixture.worktrees, fixture.presence, fixture.journals, fixture.now
    ).unwrap();
    assert!(model.lanes.iter().all(|lane| lane.chats.is_empty()));
}
```

- [ ] **Step 5: Run Dock model and acceptance tests**

Run: `cargo test --test dock_model --test live_dock_acceptance`

Expected: PASS, including deterministic ordering and no raw-content canary leakage.

- [ ] **Step 6: Commit the read-model slice after TG0 message confirmation**

Stage: `git add src/dock.rs tests/dock_model.rs tests/live_dock_acceptance.rs tests/support/mod.rs`

Proposed message: `[FEAT](dock): Project unified workspace lanes`

### Task 3: MCP-Owned Right-Browser Reopen Tool

**Files:**
- Modify: `src/viewer.rs`
- Modify: `src/mcp.rs`
- Modify: `tests/dock_viewer.rs`
- Modify: `tests/dock_mcp.rs`
- Modify: `tests/mcp_stdio.rs`

**Interfaces:**
- Consumes: `start_live_viewer(&Path, SocketAddr) -> Result<(ViewerHandle, ViewerRuntime), DevMapError>` and the existing MCP tool dispatch.
- Produces: `DOCK_BROWSER_TOOL`, a seven-entry `MCP_TOOLS`, `ViewerHandle::url() -> String`, `ViewerRuntime::is_running() -> bool`, and process-owned reuse/cleanup through `McpRuntime::browser_dock`.

- [ ] **Step 1: Write MCP lifecycle tests before changing runtime code**

```rust
#[test]
fn browser_tool_is_the_only_dock_tool_that_opens_and_reuses_a_listener() {
    let repo = committed_repo();
    let mut runtime = McpRuntime::open(repo.path()).unwrap();
    call(&mut runtime, DOCK_DATA_TOOL);
    call(&mut runtime, DOCK_RENDER_TOOL);
    assert_eq!(runtime.audit().tcp_listeners_opened, 0);
    let first = call(&mut runtime, DOCK_BROWSER_TOOL);
    let second = call(&mut runtime, DOCK_BROWSER_TOOL);
    assert_eq!(runtime.audit().tcp_listeners_opened, 1);
    assert_eq!(first["result"]["structuredContent"]["url"], second["result"]["structuredContent"]["url"]);
    assert_eq!(first["result"]["structuredContent"]["reused"], false);
    assert_eq!(second["result"]["structuredContent"]["reused"], true);
    assert!(!first["result"]["content"][0]["text"].as_str().unwrap().contains("token="));
}
```

- [ ] **Step 2: Run the lifecycle test and observe the unknown-tool failure**

Run: `cargo test --test dock_mcp browser_tool_is_the_only_dock_tool_that_opens_and_reuses_a_listener -- --exact`

Expected: FAIL because `DOCK_BROWSER_TOOL` is undefined.

- [ ] **Step 3: Make Viewer ownership observable and safely droppable**

Add `ViewerHandle::url`, have the worker set a shared `running: AtomicBool` false on exit, expose `ViewerRuntime::is_running`, and implement `Drop` to set shutdown and unblock the server. Keep explicit `shutdown(self)` for tests, joining exactly once; do not add a test-only production cleanup method.

- [ ] **Step 4: Implement `devmap_start_browser_dock` in mutable MCP dispatch**

Set `pub const DOCK_BROWSER_TOOL: &str = "devmap_start_browser_dock"`, extend `MCP_TOOLS` to seven entries, and change `call_tool_response` to receive the mutable `McpRuntime` state needed to start or reuse:

```rust
struct BrowserDock { handle: ViewerHandle, runtime: ViewerRuntime }

pub struct McpRuntime {
    workspace: SourceWorkspace,
    dock: Option<DockService>,
    browser_dock: Option<BrowserDock>,
    audit: TransportAudit,
    legacy_initialized: bool,
}
```

On the first call bind `127.0.0.1:0`, increment `tcp_listeners_opened` only after success, refresh a Dock snapshot for `revision`, and return `structuredContent: { url, revision, reused: false }`. If the saved runtime is healthy, return the same URL with `reused: true`; if it ended, discard it and start a fresh instance. Text content says only `DevMap Browser Dock is ready.` and never contains the URL or token.

- [ ] **Step 5: Verify runtime drop closes the authenticated endpoint**

Add a real HTTP test that calls the tool, confirms `/api/v1/health` returns 200 with the token, drops `McpRuntime`, retries for at most one second, and asserts the connection fails. This tests ownership at the network boundary rather than a mock.

- [ ] **Step 6: Run MCP, STDIO, and Viewer suites**

Run: `cargo test --test dock_mcp --test mcp_stdio --test dock_viewer`

Expected: PASS; existing snapshot/render calls still leave `tcp_listeners_opened == 0`.

- [ ] **Step 7: Commit the Browser bridge slice after TG0 message confirmation**

Stage: `git add src/viewer.rs src/mcp.rs tests/dock_viewer.rs tests/dock_mcp.rs tests/mcp_stdio.rs`

Proposed message: `[FEAT](dock): Add reusable right browser bridge`

### Task 4: Responsive Git Relationship Map Frontend

**Files:**
- Modify: `assets/dock.html`
- Modify: `tests/dock_ui_contract.rs`
- Modify: `tests/live_dock_acceptance.rs`

**Interfaces:**
- Consumes: `DockReadModel::development_target`, `DockReadModel::lanes`, `DockLane::relationship`, and `DockLane::chats` from Task 2 through either MCP tool results or Viewer SSE.
- Produces: one accessible `.relationship-map` with `.target-left`, `.workspace-node`, `.chat-node`, `.return-edge`, and `.target-right` per lane.

- [ ] **Step 1: Replace card-oriented contract assertions with graph behavior assertions**

```rust
#[test]
fn dock_asset_renders_each_lane_through_the_repeated_merge_target() {
    let html = dock_html();
    for contract in ["relationship-map", "target-left", "workspace-node", "chat-node", "return-edge", "target-right"] {
        assert!(html.contains(contract), "missing graph contract: {contract}");
    }
    assert!(html.contains("Merged into"));
    assert!(html.contains("Not merged"));
    assert!(html.contains("No linked chat"));
    assert!(html.contains("ahead"));
    assert!(html.contains("behind"));
    assert!(!html.contains("<details class=\"group\""));
}
```

- [ ] **Step 2: Run the UI contract test and confirm it fails on the old cards**

Run: `cargo test --test dock_ui_contract dock_asset_renders_each_lane_through_the_repeated_merge_target -- --exact`

Expected: FAIL because the graph nodes are absent.

- [ ] **Step 3: Implement the wide map and semantic color system**

Use CSS Grid columns `minmax(72px,.65fr) minmax(180px,1.8fr) minmax(150px,1.25fr) minmax(90px,.75fr)` for each lane. Render the same target label on the left and right. Use green only for `merged === true`, amber only for `merged === false`, muted gray for unknown/unlinked, purple for working, cyan for idle, blue for waiting, and a separate red dirty dot with changed-file count. Edges are CSS/SVG presentation; every state also has visible text so color is never the sole signal.

- [ ] **Step 4: Implement narrow layout, keyboard focus, and safe model handling**

At `max-width: 519px`, switch the lane to one column in the exact order base target, workspace, chat, merge target. Retain `aria-live`, `:focus-visible`, `prefers-reduced-motion`, `textContent`, `replaceChildren`, schema/field bounds, monotonic revision checks, offline timing, and portable model-context updates. Set `min-width: 0`, wrap paths, and prevent document-level horizontal overflow.

- [ ] **Step 5: Run contract and Browser acceptance tests**

Run: `cargo test --test dock_ui_contract --test live_dock_acceptance`

Expected: PASS, asset size remains below 128 KiB, and canary strings remain absent.

- [ ] **Step 6: Perform real responsive visual QA**

Start the disposable Viewer with `cargo run -- view --source <fixture-repository> --live`, open the authenticated local URL in the Codex Browser panel, and inspect 320, 360, 520, 736, and 1,024 px widths. Confirm current lane first, right target visible on every lane, merged edge reaches it, unmerged edge stops before it, labels do not overlap, keyboard focus is visible, and there is no horizontal page scrollbar. Save screenshots only under the ignored `.superpowers/brainstorm/` directory.

- [ ] **Step 7: Commit the frontend slice after TG0 message confirmation**

Stage: `git add assets/dock.html tests/dock_ui_contract.rs tests/live_dock_acceptance.rs`

Proposed message: `[FEAT](dock): Render the Git relationship map`

### Task 5: Plugin Reopen Workflow and Installed Cachebuster

**Files:**
- Modify: `plugins/devmap/skills/live-worktree-dock/SKILL.md`
- Modify: `plugins/devmap/.mcp.json`
- Modify: `plugins/devmap/.codex-plugin/plugin.json`
- Modify: `tests/dock_plugin.rs`
- Modify: personal marketplace metadata only through `plugin-creator` scripts

**Interfaces:**
- Consumes: MCP tool `devmap_start_browser_dock` and Codex app opener `{ target: { type: "browser", url }, placement: "right" }`.
- Produces: a documented plugin workflow that opens the embedded app normally and uses one start-tool call plus one Browser opener call for explicit right-side open/reopen requests.

- [ ] **Step 1: Write plugin behavior tests for the third Dock tool**

Update the parsed JSON assertions to require three auto-approved Dock tools and the exact Browser tool name:

```rust
assert_eq!(config["mcpServers"]["devmap"]["autoApprove"].as_array().unwrap(), &[
    json!("devmap_dock_snapshot"),
    json!("devmap_open_dock"),
    json!("devmap_start_browser_dock"),
]);
```

Also assert the manifest remains valid and the default prompt requests the Git relationship map without promising native panel persistence.

- [ ] **Step 2: Run the plugin test and observe the two-tool failure**

Run: `cargo test --test dock_plugin`

Expected: FAIL because the Browser tool is not registered in plugin configuration.

- [ ] **Step 3: Update plugin instructions and metadata**

Teach the skill these branches:

```text
Normal open/show/refresh -> call devmap_open_dock.
Explicit right-side open/reopen -> call devmap_start_browser_dock once, then open structuredContent.url with the Codex Browser at placement right.
Never repeat the authenticated URL in prose. Never launch a manual terminal server for the Codex path.
If the opener returns queued, report queued and do not start another Viewer.
```

Add `devmap_start_browser_dock` to `.mcp.json` auto approval and change the default prompt to `Open the DevMap Git relationship map on the right.` Keep manifest command and icon identities stable.

- [ ] **Step 4: Run plugin and complete Cargo suites**

Run: `cargo test --test dock_plugin && cargo test --all-targets`

Expected: PASS.

- [ ] **Step 5: Update the personal plugin cachebuster through the required scripts**

From the `plugin-creator` skill directory, run:

```powershell
python scripts/read_marketplace_name.py
python scripts/update_plugin_cachebuster.py 'C:/Users/user/Documents/ChatGPT/AI auto-git context/.worktrees/devmap-live-worktree-dock/plugins/devmap'
codex plugin add devmap@personal
```

Do not hand-edit marketplace metadata. Verify the installed version path changes and contains the new skill/config/frontend. Test the updated plugin in a new Codex task because the current task keeps the already-loaded skill version.

- [ ] **Step 6: Commit the plugin slice after TG0 message confirmation**

Stage the repository-owned plugin files and any script-generated repository metadata reported by `git status`.

Proposed message: `[FEAT](plugin): Add right-side DevMap reopen workflow`

### Task 6: Final Verification, Documentation, and Push

**Files:**
- Modify only when verification finds a concrete defect: files from Tasks 1–5
- Verify: `README.md`
- Verify: `docs/superpowers/specs/2026-09-02-devmap-live-worktree-dock-design.md`
- Verify: `docs/superpowers/plans/2026-09-03-devmap-git-relationship-map.md`

**Interfaces:**
- Consumes: all prior task outputs.
- Produces: a clean, tested branch pushed to `origin/codex/devmap-live-worktree-dock`, plus a reproducible right-Browser open/reopen validation result.

- [ ] **Step 1: Run formatting and static verification**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: both exit 0.

- [ ] **Step 2: Run the full automated test suite**

Run: `cargo test --all-targets`

Expected: exit 0 with no failed, ignored, or filtered tests in the full run.

- [ ] **Step 3: Verify read-only and privacy boundaries**

Run the acceptance snapshot helper before and after Dock snapshot, embedded-App, and Browser-tool calls. Assert source files, Git index, refs, config, stash, remotes, and worktree metadata are unchanged. Search serialized Dock responses for the canaries `tool_input`, `tool_output`, `transcript`, raw prompts, and authenticated `token=` in text content; all must be absent.

- [ ] **Step 4: Verify the real reopen workflow in a fresh task**

Open a new Codex task with the updated plugin, request `Open DevMap on the right`, confirm one Browser tab appears on the right, close it, repeat the request, and confirm the Viewer URL is reused while healthy. End that task and confirm the endpoint stops accepting connections.

- [ ] **Step 5: Review the final diff and branch state**

Run: `git status --short`

Run: `git diff --check`

Run: `git log --oneline --decorate -8`

Run: `git diff origin/main...HEAD --stat`

Expected: no unintended files, no whitespace errors, and only approved DevMap Dock changes.

- [ ] **Step 6: Commit any verification-only fix after TG0 message confirmation**

If Step 1–5 required a repository change, stage only that fix and use proposed message `[FIX](dock): Resolve final verification findings`. If no files changed, do not create an empty commit.

- [ ] **Step 7: Push the completed branch**

Run: `git push origin codex/devmap-live-worktree-dock`

Expected: the remote branch advances to the verified local HEAD without force-push.
