# DevMap Branch Topology and Task Refresh Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace DevMap's flat worktree lanes with a compact, exact Git hierarchy (`main`/`master` → `dev`/`develop` → feature worktrees), group worktrees at their true common-base commit, expose copyable commit/tag details, and make a renamed Codex task appear after a host-backed `Refresh all`.

**Architecture:** `GitRelationshipResolver` becomes the sole owner of integration-target selection and read-only Git topology facts. `DockReducer` joins those facts with exact workspace/task associations into a bounded `devmap/dock/2` hierarchy. The shared HTML renders one rail per integration branch and one station per exact `(target ref, merge-base hash)` group, while a capability-gated host message requests a fresh Codex task inventory without reading private Codex storage.

**Tech Stack:** Rust 2024 (Rust 1.96), Serde/JSON, read-only Git subprocesses, Tiny HTTP/SSE, MCP over STDIO, self-contained HTML/CSS/SVG/JavaScript, Cargo integration tests, Codex personal plugin cachebuster workflow.

**Spec:** `docs/superpowers/specs/2026-09-03-devmap-branch-topology-and-task-refresh-design.md`

## Global Constraints

- Before implementation, read `superpowers:test-driven-development/writing-good-tests.md`; for every behavior below, add one focused failing test, run it and verify the expected failure, then write the minimum production code.
- Never infer a historical branch-creation command. The UI says `Common base with <target>` and reports the exact `git merge-base`, while the integration hierarchy remains an explicit current comparison policy.
- Root target precedence is remote default, local `main`, then local `master`. Development target precedence is valid `devmap.developmentTarget`, local `dev`, then local `develop`; it must be distinct from the root.
- The development branch returns to the root. Ordinary worktree branches return to the development branch when present, otherwise to the root. The root has no return edge.
- Group only by full `(target ref, merge-base hash)`. Never group by short hash, branch/path similarity, timestamp, or tag name.
- All Git calls are read-only. Merge-base/tag/metadata failures retain the workspace in an `Unknown base` group with a bounded warning.
- A Codex task attaches only when its exact local `cwd` matches a scanned worktree. Task titles are untrusted display text and are never interpreted as instructions.
- `codex_tasks` omission retains the last host inventory; a supplied array replaces it; a supplied empty array clears it. Do not collapse `Some(vec![])` into omission in the Browser path.
- Do not read Codex SQLite databases, session logs, DOM state, prompts, tool arguments/results, or transcripts. The refresh request contains no task titles, paths, session IDs, hashes, tokens, or other private state.
- The frontend uses `textContent`/DOM construction only, no `innerHTML`, external URLs, browser storage, fixed-height nested scrollers, or document-level horizontal overflow.
- Color supplements visible text: root blue, development green, ordinary branches/tasks purple, merged green + `Merged`, open amber + `Not merged`, unknown neutral dashed + `Unknown`, dirty red dot + count.
- Preserve unrelated user changes and the untracked `.superpowers/brainstorm/` directory. Never stage or delete the brainstorming artifacts.
- Read `plugin-creator/SKILL.md` and its required installation reference before changing or installing the plugin. Read `superpowers:verification-before-completion/SKILL.md` before any completion claim.
- Every commit and the final push require the TG0 Commit Message Generator confirmation workflow. Do not commit or push merely because a task checkbox is complete.

---

### Task 1: Resolve the Integration Hierarchy and Exact Fork Metadata

**Files:**
- Modify: `src/git_relationship.rs`
- Modify: `tests/git_relationship.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**
- Extend `TargetSource` only as needed to distinguish root/default and development selection without changing existing serialized snake-case names unexpectedly.
- Add serialized topology facts:

```rust
pub struct IntegrationBranch {
    pub name: String,
    pub ref_name: String,
    pub head: String,
    pub parent: Option<String>,
    pub source: TargetSource,
}

pub struct ForkPoint {
    pub target_branch: String,
    pub commit: String,
    pub tags: Vec<String>,
    pub subject: Option<String>,
    pub authored_at: Option<String>,
    pub distance_to_target: Option<u32>,
}

pub struct GitRelationship {
    // retain merge_target, merged, ahead, behind, dirty, changed_file_count
    pub fork_point: Option<ForkPoint>,
}

pub struct GitRelationshipReport {
    pub integration_branches: Vec<IntegrationBranch>,
    pub by_worktree_id: BTreeMap<String, GitRelationship>,
    pub warnings: Vec<GitRelationshipWarning>,
}
```

- Keep compatibility access to `DevelopmentTarget` only if an existing non-frontend caller still needs it; do not maintain two independent target-selection implementations.

- [ ] **Step 1: Add a four-branch hierarchy test**

Create `main`, `dev`, `dylan_test`, and `Joe_dev` in a disposable repository, with `dev` one commit ahead of `main` and both feature branches created from `dev`. Assert:

```rust
assert_eq!(parent_of(&report, "dev"), Some("main"));
assert_eq!(target_of(&report, "dylan_test"), Some("dev"));
assert_eq!(target_of(&report, "Joe_dev"), Some("dev"));
assert_eq!(target_of(&report, "main"), None);
```

- [ ] **Step 2: Run the focused test and observe the old single-target behavior**

Run: `cargo test --test git_relationship hierarchy_routes_features_to_dev_and_dev_to_main -- --exact`

Expected: FAIL because the report has one flat development target and no parent hierarchy.

- [ ] **Step 3: Split root and development target selection**

Implement small, deterministic helpers:

```rust
fn select_root_target(workspace: &SourceWorkspace) -> Result<Option<DevelopmentTarget>, DevMapError>;
fn select_development_target(
    workspace: &SourceWorkspace,
    root: Option<&DevelopmentTarget>,
    warnings: &mut Vec<GitRelationshipWarning>,
) -> Result<Option<DevelopmentTarget>, DevMapError>;
fn target_for_worktree<'a>(
    worktree: &WorktreeDescriptor,
    root: Option<&'a DevelopmentTarget>,
    development: Option<&'a DevelopmentTarget>,
) -> Option<&'a DevelopmentTarget>;
```

Resolve each selected ref to a full commit with `git rev-parse --verify <ref>^{commit}`. Treat a worktree checked out on the root branch as terminal, the development branch as targeting root, and every other branch as targeting development-or-root.

- [ ] **Step 4: Add exact merge-base, tag, subject, and date tests**

Create two features at one `dev` commit, tag that commit, then advance `dev`. Assert both relationships contain the same full merge-base hash, the tag appears exactly once in sorted order, the bounded subject and RFC 3339 authored time match the common-base commit, and `distance_to_target` places the station behind the advanced `dev` head.

Add a second tag on another commit and assert it is absent. Add a third feature from the advanced `dev` head and assert its merge-base differs.

- [ ] **Step 5: Run metadata tests and observe missing fields**

Run: `cargo test --test git_relationship fork_point_contains_exact_commit_metadata_and_tags -- --exact`

Run: `cargo test --test git_relationship exact_tags_do_not_leak_from_other_commits -- --exact`

Expected: FAIL at compile time or assertions because `ForkPoint` is not populated.

- [ ] **Step 6: Implement bounded, deduplicated Git probes**

For each unique `(target.ref_name, worktree.head)`:

```text
git merge-base <target-ref> <head>
git tag --points-at <merge-base>
git show -s --format=%s%n%aI <merge-base>
git rev-list --count <merge-base>..<target-ref>
git rev-list --left-right --count <target-ref>...<head>
```

Reuse the existing bounded worker strategy. Cap tags and strings with named constants, sort/deduplicate tags, validate full object IDs, and retain exact ahead/behind merge semantics (`ahead == 0` means merged into the immediate target).

- [ ] **Step 7: Add and implement unknown-base behavior**

Make a relationship probe fail using an unavailable target/ref fixture. Assert the worktree remains in `by_worktree_id`, `fork_point == None`, `merged == None`, and a `git_merge_base_unavailable` warning names its worktree ID. Keep dirty-state facts when ancestry metadata fails.

Run: `cargo test --test git_relationship merge_base_failure_retains_unknown_workspace -- --exact`

Expected before implementation: FAIL because the current fallback loses the new topology facts.

- [ ] **Step 8: Run the resolver regression suite**

Run: `cargo test --test git_relationship --test worktrees`

Expected: PASS, including existing configured-target, ahead/behind, dirty, and rename-count tests.

- [ ] **Step 9: Review and request TG0 commit confirmation**

Stage only `src/git_relationship.rs`, `tests/git_relationship.rs`, and `tests/support/mod.rs` after approval.

Proposed message: `[FEAT](dock): Resolve hierarchical Git fork topology`

### Task 2: Build the Grouped Dock v2 Read Model

**Files:**
- Modify: `src/dock.rs`
- Modify: `tests/dock_model.rs`
- Modify: `tests/live_dock_acceptance.rs`
- Modify: `tests/support/mod.rs`

**Interfaces:**
- Set `DOCK_SCHEMA_VERSION` to `devmap/dock/2`.
- Add:

```rust
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BranchGroup {
    pub target_branch: String,
    pub fork_point: Option<ForkPoint>,
    pub lanes: Vec<DockLane>,
}

pub struct DockReadModel {
    // existing identity/revision/warning fields
    pub integration_branches: Vec<IntegrationBranch>,
    pub branch_groups: Vec<BranchGroup>,
    pub task_inventory_synced_at: Option<String>,
    // retain compatibility arrays only for text/older callers during this release
}
```

- `DockReducer` receives topology facts from `GitRelationshipResolver`; it does not shell out or infer ancestry from names.

- [ ] **Step 1: Add a same-fork grouping test**

Construct two feature worktrees with target `dev` and the same full fork hash. Assert one `BranchGroup`, two deterministically ordered lanes, one station hash, and one `dev` integration rail.

- [ ] **Step 2: Run the focused grouping test**

Run: `cargo test --test dock_model reducer_groups_worktrees_at_the_same_exact_fork_point -- --exact`

Expected: FAIL because `DockReadModel` still exposes only flat `lanes`.

- [ ] **Step 3: Add a distinct-and-ordered station test**

Construct two groups at different commits on `dev`. Assert earlier `distance_to_target` values render farther from the target, ties break by full hash, and current-lane ordering is applied only inside a station rather than moving the station itself.

Run: `cargo test --test dock_model reducer_keeps_distinct_fork_points_in_history_order -- --exact`

Expected: FAIL because station grouping and ordering do not exist.

- [ ] **Step 4: Implement deterministic grouping and schema v2 serialization**

Use a `BTreeMap<(String, Option<String>), Vec<DockLane>>`, where keys are the exact target ref/name and full fork hash; create a separate unknown group per target. Sort integration branches root-first, groups by target rail and `distance_to_target`, and lanes current-first then branch/path/worktree ID.

Include `integration_branches`, `branch_groups`, lane contents, task titles/status, warnings, and `task_inventory_synced_at` in `content_hash`. Exclude `generated_at`. Keep revision monotonic and unchanged for ordinary polling with identical Git/task facts.

- [ ] **Step 5: Make truncation group-aware**

Update `bound_model` so it first retains the root/development rails and at least one bounded representation of each affordable workspace, then trims extra chats/tags/metadata. Never leave a group referencing a removed rail, silently drop an entire current workspace, or exceed `MAX_DOCK_MODEL_BYTES`; set `truncated` and a visible warning when detail is removed.

- [ ] **Step 6: Update acceptance/schema assertions**

Change literal schema assertions from `devmap/dock/1` to `devmap/dock/2`. Add a privacy canary assertion across `branch_groups` and details, plus a deterministic canonical JSON check for repeated reductions.

- [ ] **Step 7: Run Dock model and acceptance suites**

Run: `cargo test --test dock_model --test live_dock_acceptance`

Expected: PASS with bounded output and no source Git mutation.

- [ ] **Step 8: Review and request TG0 commit confirmation**

Stage only the Task 2 files after approval.

Proposed message: `[FEAT](dock): Group worktrees by exact fork station`

### Task 3: Replace and Refresh the Codex Task Inventory Correctly

**Files:**
- Modify: `src/dock.rs`
- Modify: `src/mcp.rs`
- Modify: `src/viewer.rs`
- Modify: `tests/dock_model.rs`
- Modify: `tests/dock_mcp.rs`
- Modify: `tests/dock_viewer.rs`

**Interfaces:**
- Replace `DockService::set_observed_tasks` with explicit replacement semantics:

```rust
pub fn replace_observed_tasks(
    &mut self,
    tasks: Vec<ObservedTask>,
    synced_at: OffsetDateTime,
) -> Result<&DockReadModel, DevMapError>;
```

- Normalize and compare the complete inventory. Update `task_inventory_synced_at` only when the normalized inventory differs, so repeated identical host refreshes do not create revision churn.
- Preserve the `Option<Vec<ObservedTask>>` distinction through MCP and Viewer calls.

- [ ] **Step 1: Add a renamed-session regression test**

Supply session `abc` with title `Old title`, then replace the inventory with the same session/path/status and title `New title`. Assert one task remains, its displayed title changes, `task_inventory_synced_at` advances, and the model revision increments once.

Run: `cargo test --test dock_model replacing_inventory_updates_a_renamed_task_title -- --exact`

Expected: FAIL until sync metadata and explicit replacement behavior are implemented.

- [ ] **Step 2: Add omit/empty/identical inventory tests at the MCP boundary**

Exercise `devmap_dock_snapshot` and `devmap_start_browser_dock` in sequence:

1. supplied one-row array stores one task;
2. omitted `codex_tasks` retains it;
3. identical supplied array retains the revision/sync marker;
4. supplied `[]` clears it and advances revision;
5. omitted array after clearing remains empty.

Run: `cargo test --test dock_mcp codex_task_inventory_distinguishes_omit_replace_and_clear -- --exact`

Expected: FAIL specifically on Browser `[]`, because current code converts empty inventory into Git-only refresh.

- [ ] **Step 3: Implement normalized replacement state**

Sort tasks by session ID and reject duplicate session IDs instead of allowing order-dependent joins. Keep existing size/host/kind/status/ID/path/title/time validation. Store normalized tasks plus their last meaningful sync time in `DockService`; refresh Git independently without mutating that state.

- [ ] **Step 4: Propagate exact `Option` semantics into a running Viewer**

Rename `ViewerRuntime::set_observed_tasks` to `replace_observed_tasks`. In `start_or_reuse_browser_dock`, match explicitly on `None` versus `Some(tasks)` and always forward `Some(vec![])`. Ensure the standalone Viewer HTTP refresh recomputes Git while retaining inventory and the shared MCP-owned Viewer receives later replacements.

- [ ] **Step 5: Add Viewer propagation coverage**

Start the Viewer with one observed task, replace it with the same session and a new title through the runtime, fetch `/api/v1/snapshot`, and assert the new title and revision. Then replace with empty and assert the task disappears without restarting the listener.

Run: `cargo test --test dock_viewer viewer_applies_renamed_and_cleared_task_inventory -- --exact`

Expected before implementation: FAIL because the v2 sync state and clear path are absent.

- [ ] **Step 6: Run MCP, Viewer, and reducer regressions**

Run: `cargo test --test dock_model --test dock_mcp --test dock_viewer --test mcp_stdio`

Expected: PASS; remote/unsupported tasks and oversized fields remain rejected or excluded as before.

- [ ] **Step 7: Review and request TG0 commit confirmation**

Stage only the Task 3 files after approval.

Proposed message: `[FIX](dock): Refresh renamed Codex task titles`

### Task 4: Render the Compact Hierarchical Map and Detail Inspector

**Files:**
- Modify: `assets/dock.html`
- Modify: `tests/dock_ui_contract.rs`
- Modify: `tests/live_dock_acceptance.rs`

**Interfaces:**
- Consume only `devmap/dock/2`, `integration_branches`, and `branch_groups`; do not reconstruct target ancestry in JavaScript.
- Render one semantic `.integration-rail` per integration branch, one `.fork-station` per group, nested `.workspace-branch` rows, `.task-node` buttons, `.return-state` labels, and one shared `#selection-details` region.

- [ ] **Step 1: Replace the flat-lane contract test with hierarchy contracts**

Require the asset to contain `integration-rail`, `fork-station`, `workspace-branch`, `task-node`, `selection-details`, `Copy hash`, `Merged`, `Not merged`, `Unknown`, and `No exact tag`. Assert the old repeated `.target-left`/`.target-right` flat-lane contract is absent.

Run: `cargo test --test dock_ui_contract dock_asset_renders_shared_integration_rails_and_fork_stations -- --exact`

Expected: FAIL because the current asset repeats a target for every lane.

- [ ] **Step 2: Implement safe v2 validation and graph DOM construction**

Validate bounds for every rail, group, fork point, lane, and task before accepting a snapshot. Construct labels and details with `createElement`, `textContent`, `replaceChildren`, and button attributes. Reject a whole malformed snapshot and retain the last valid render; never partially trust a nested group.

- [ ] **Step 3: Implement the main → dev → feature visual hierarchy**

Use one root rail and a nested development section. Shared fork hashes render once with the worktree count and fan-out connector; different hashes render as separate stations ordered by the model. Root/development rails remain visually continuous, while workspace/task rows stay compact and card-free. Each return edge names its immediate target.

- [ ] **Step 4: Add progressive selection details and copy feedback**

Make station, workspace, and task rows native buttons. Station selection shows full common-base hash, exact tags or `No exact tag`, subject/date, target, and attached count. Workspace selection shows full path/HEAD, target, fork hash, ahead/behind, merge state, dirty count, and tasks. Task selection shows full title, local session ID, association source, and host state.

Use `navigator.clipboard.writeText(fullHash)` only from the `Copy hash` click. On success set the control text to `Copied`; on failure use `Copy unavailable` while keeping the hash selectable. Announce both through the existing polite live region.

- [ ] **Step 5: Add accessible responsive and no-injection assertions**

Test for native buttons, `:focus-visible`, `aria-live="polite"`, `prefers-reduced-motion`, coarse-pointer 44 px targets, `overflow-wrap`, and a narrow vertical tree breakpoint below 620 px. Continue asserting no `innerHTML`, `eval`, external URL, storage API, or interpolated HTML sink.

- [ ] **Step 6: Run UI and acceptance tests**

Run: `cargo test --test dock_ui_contract --test live_dock_acceptance`

Expected: PASS and the self-contained asset remains below 128 KiB.

- [ ] **Step 7: Perform real Browser QA at four widths**

Open the authenticated local Viewer in the Codex Browser and inspect 375, 594, 736, and 1,024 px. Verify the `main → dev → dylan_test/Joe_dev` shape, shared-station fan-out, no overlapping labels, no document horizontal scrollbar, visible keyboard focus, selection detail updates, successful hash copy, and zero console errors.

Save any screenshots only under ignored `.superpowers/brainstorm/`.

- [ ] **Step 8: Review and request TG0 commit confirmation**

Stage only the Task 4 files after approval.

Proposed message: `[FEAT](dock): Render hierarchical branch topology`

### Task 5: Add Host-Backed `Refresh all` and Update the Plugin Workflow

**Files:**
- Modify: `assets/dock.html`
- Modify: `tests/dock_ui_contract.rs`
- Modify: `plugins/devmap/skills/live-worktree-dock/SKILL.md`
- Modify: `plugins/devmap/.codex-plugin/plugin.json`
- Modify: `tests/dock_plugin.rs`
- Update personal marketplace/install metadata only through `plugin-creator` scripts

**Interfaces:**
- UI request text is a fixed, non-sensitive English instruction such as `Refresh DevMap task inventory for the current local repository and update the open Dock.`
- Prefer `await window.openai.sendFollowUpMessage({ prompt, title: "Refresh DevMap" })` when exposed by the Codex Browser host. In an MCP App, send the equivalent portable `ui/message` request through the existing JSON-RPC parent bridge.
- If neither capability exists, run the existing Git-only snapshot/fetch and show `Git refreshed · task names not resynced` plus `Ask Codex: Refresh DevMap`.

- [ ] **Step 1: Read plugin-authoring instructions before edits**

Read `plugin-creator/SKILL.md` completely and every installation/cachebuster reference it requires. Record the supported script commands; do not hand-edit personal marketplace metadata.

- [ ] **Step 2: Add UI contract tests for capability-gated refresh**

Require `Refresh all`, `Requesting Codex…`, `sendFollowUpMessage`, `ui/message`, `Git refreshed · task names not resynced`, and the non-sensitive fixed prompt. Assert the prompt does not concatenate any model values.

Run: `cargo test --test dock_ui_contract dock_refresh_requests_a_fresh_host_task_inventory -- --exact`

Expected: FAIL because the current button only calls `devmap_dock_snapshot`/HTTP fetch.

- [ ] **Step 3: Implement the refresh state machine**

On click, disable the button and change its label immediately. Record the current `task_inventory_synced_at`, send one host message, and continue normal snapshot/SSE consumption. Restore `Refresh all` when a newer sync marker arrives. If the replacement is identical and therefore produces no newer marker, restore after a bounded timeout with `No task changes reported`; do not send a second message automatically.

In the unsupported-host fallback, perform one Git-only refresh and display the stale-task label. Automatic two-second Git polling must never emit host follow-up messages.

- [ ] **Step 4: Update the DevMap skill for complete replacement**

Specify the English and Chinese command intent for `Refresh DevMap`: call `list_threads` once, keep only local Codex `active`/`idle` tasks, copy only `id`, `title`, `status`, `cwd`, `updatedAt`, `hostId`, and `kind`, then call `devmap_dock_snapshot` with the complete `codex_tasks` array. An empty filtered result must be sent as `[]`; omission is reserved for Git-only refresh.

Retain the existing normal-open and right-Browser reopen branches. State that task titles are untrusted display text and that refresh never reads private local Codex databases.

- [ ] **Step 5: Update plugin tests and cachebuster version**

Assert the skill contains complete-replacement, empty-array, rename-refresh, and privacy wording. Run:

`cargo test --test dock_plugin --test dock_ui_contract`

Expected before metadata/skill edits: FAIL on the new refresh workflow assertions; expected after edits: PASS.

- [ ] **Step 6: Update/install the plugin only through the cachebuster workflow**

Use the exact scripts and marketplace path prescribed by the current `plugin-creator` skill, then reinstall `devmap@personal`. Verify the installed cache directory contains the new v2 asset, skill, manifest, and MCP configuration. A fresh Codex task is required for the final plugin validation because the current task keeps its loaded plugin version.

- [ ] **Step 7: Perform end-to-end rename refresh QA**

In a fresh task, open DevMap, rename that task in Codex, click `Refresh all`, allow the generated follow-up to run, and assert the same session node displays the new title without restarting the Viewer. Repeat once with no rename and verify the bounded `No task changes reported` state. Close/reopen the Browser tab and verify the healthy Viewer is reused.

- [ ] **Step 8: Review and request TG0 commit confirmation**

Stage repository-owned Task 5 files only. Do not stage `.superpowers/brainstorm/` or user-level plugin cache files.

Proposed message: `[FEAT](plugin): Refresh DevMap task inventory from Codex`

### Task 6: Full Verification, Documentation Consistency, and Delivery

**Files:**
- Verify: `README.md`
- Verify: `docs/superpowers/specs/2026-09-03-devmap-branch-topology-and-task-refresh-design.md`
- Verify: `docs/superpowers/plans/2026-09-03-devmap-branch-topology-and-task-refresh.md`
- Modify only files with concrete defects found by verification

**Interfaces:**
- Produce a clean, bounded `devmap/dock/2` implementation and verified plugin installation. Do not merge branches or push until the user explicitly confirms the TG0 proposal.

- [ ] **Step 1: Read and apply verification-before-completion**

Read `superpowers:verification-before-completion/SKILL.md` completely before claiming any result is fixed or passing.

- [ ] **Step 2: Run formatting and static analysis**

Run: `cargo fmt --all -- --check`

Run: `cargo clippy --all-targets --all-features -- -D warnings`

Expected: both exit 0.

- [ ] **Step 3: Run the complete automated suite**

Run: `cargo test --all-targets`

Expected: exit 0 with no failures. Record the actual test count from fresh output; do not reuse earlier evidence.

- [ ] **Step 4: Recheck privacy, bounds, and read-only guarantees**

Run `git diff --check`. Exercise snapshot, embedded App, Browser Viewer, and refresh paths against canaries named `tool_input`, `tool_output`, `transcript`, and a fake prompt. Assert none appear in serialized models/UI, authenticated `token=` appears only in structured Browser URL output, and source files/index/refs/config/stash/remotes/worktree metadata are unchanged.

- [ ] **Step 5: Repeat responsive Browser QA on real repository data**

Inspect 375, 594, 736, and 1,024 px with the repository's actual worktrees. Confirm task titles `Dev agent` and `Testing agent` are not truncated beyond recognition, exact station hash copies, different branch points do not share a station, merged/open states target the immediate integration branch, and the `main` terminal is visible without repeated clutter.

- [ ] **Step 6: Review final branch contents**

Run:

```text
git status --short --branch
git diff --check
git log --oneline --decorate -12
git diff origin/codex/devmap-live-worktree-dock...HEAD --stat
```

Expected: only approved source/tests/docs/plugin changes plus the intentionally untracked `.superpowers/brainstorm/` directory.

- [ ] **Step 7: Request final TG0 commit/push confirmation**

If verification created an additional fix, propose `[FIX](dock): Resolve topology verification findings`; otherwise do not create an empty commit. Present the complete commit list and push target before running:

`git push origin codex/devmap-live-worktree-dock`

- [ ] **Step 8: Report delivery with direct artifacts**

Report the pushed commit hash, fresh format/Clippy/test evidence, Browser QA widths, rename-refresh outcome, installed plugin version, and the repository plan/spec links. If the plugin marketplace entry changed, include the required Codex `View devmap` and `Share devmap` links from the plugin-creator workflow.
