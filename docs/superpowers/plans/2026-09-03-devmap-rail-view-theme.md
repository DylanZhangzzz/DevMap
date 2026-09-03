# DevMap Rail View and Color System Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Rebuild the DevMap Dock as a topology-first horizontal Rail View with progressive Agent detail and a restrained accessible dark color system.

**Architecture:** Keep the existing bounded `devmap/dock/2` read model and transport unchanged. Replace only the self-contained HTML renderer and its contract tests: integration branches remain top rails, each worktree becomes a parallel horizontal branch lane, six priority lanes remain expanded, and MAP/READ/FULL controls expose progressively richer Agent information.

**Tech Stack:** Rust 2024 contract tests, self-contained HTML/CSS/vanilla JavaScript, Tiny HTTP/SSE Viewer, MCP App bridge.

**Spec:** `docs/superpowers/specs/2026-09-03-devmap-rail-view-theme-design.md`

## Global Constraints

- Preserve `devmap/dock/2`, all current bounded validators, `textContent`/`replaceChildren` rendering, and the no-`innerHTML` guarantee.
- Preserve refresh, SSE/MCP transport, clipboard, model-context, offline-aging, and visibility-change behavior.
- Add no dependencies, external URLs, inline SVG, storage, or backend schema changes.
- Default to MAP; keep at most six priority lanes expanded until the user reveals the remainder.
- Use one blue topology accent and small-area green/amber/red semantic states; all large surfaces remain neutral.
- Follow red-green-refactor for every behavior and run the full Cargo suite before handoff.

---

### Task 1: Lock the Rail View behavior with failing artifact and browser tests

**Files:**
- Modify: `tests/dock_ui_contract.rs`

**Interfaces:**
- Consumes: `devmap::dock_asset::dock_html() -> &'static str`
- Produces: consumer-visible HTML semantics plus browser assertions for topology, density, branch disclosure, color rendering, and safety retention.

- [ ] **Step 1: Add the topology and density contract test**

Add a test named `dock_asset_renders_parallel_branch_rails_with_progressive_density` that requires these literals:

```rust
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
] {
    assert!(html.contains(contract), "missing Rail View contract: {contract}");
}
```

Also require `integration-rail`, `task-node`, `return-state`, and `selection-details` so progressive disclosure does not delete existing behavior.

- [ ] **Step 2: Add the branch-disclosure artifact contract test**

Add `dock_asset_exposes_bounded_branch_disclosure_without_dropping_status_text` and require the served artifact to expose the disclosure control and every user-visible state label:

```rust
assert!(html.contains("collapsed-branches"));
assert!(html.contains("merged / inactive branches"));
assert!(html.contains("Merged →"));
assert!(html.contains("Not merged →"));
assert!(html.contains("Unknown →"));
assert!(html.contains("DIRTY"));
```

- [ ] **Step 3: Add the color-system artifact contract test**

Add `dock_asset_uses_neutral_surfaces_and_small_area_semantic_colors`. Require `--bg-canvas`, `--surface-raised`, `--accent`, `--success`, `--warning`, and `--danger`, which are part of the shipped HTML styling boundary. Assert the removed decorative tokens `--branch-soft` and `--dev-soft` are absent.

- [ ] **Step 4: Run the focused tests and verify RED**

Run:

```text
cargo test --test dock_ui_contract dock_asset_renders_parallel_branch_rails_with_progressive_density -- --exact
cargo test --test dock_ui_contract dock_asset_exposes_bounded_branch_disclosure_without_dropping_status_text -- --exact
cargo test --test dock_ui_contract dock_asset_uses_neutral_surfaces_and_small_area_semantic_colors -- --exact
```

Expected: each test fails because the current served artifact contains nested `.fork-group` cards and has no density or collapse controls.

- [ ] **Step 5: Run a browser RED assertion against the current Viewer**

With a valid local Viewer snapshot loaded, assert that `button[aria-label="MAP density"]`, `.branch-rail`, and `.collapsed-branches` are visible and that activating READ changes `document.documentElement.dataset.density` to `read`.

Expected: FAIL because the current Viewer has none of those observable controls or states.

### Task 2: Implement the semantic horizontal Rail View

**Files:**
- Modify: `assets/dock.html`
- Test: `tests/dock_ui_contract.rs`

**Interfaces:**
- Consumes: validated `integration_branches`, `branch_groups`, `DockLane`, and linked task arrays.
- Produces: `createRail(rail, groups)`, `createBranchRail(lane, group)`, `rankLane(lane)`, `setDensity(mode)`, and `toggleCollapsedBranches(section, button)`.

- [ ] **Step 1: Replace the visual tokens and static shell**

Define the graphite color tokens from the spec, set `<html data-density="map">`, add `.density-switch` with three native buttons, and keep the existing summary, connection, refresh, and warning regions. Use neutral surfaces for the graph, node, and inspector.

- [ ] **Step 2: Implement lane ranking and bounded disclosure**

Add:

```javascript
const MAX_VISIBLE_BRANCHES = 6;
function activeTaskCount(lane) {
  return lane.chats.filter((task) => ["starting", "working", "waiting"].includes(task.status)).length;
}
function rankLane(lane) {
  return [
    lane.is_current ? 0 : 1,
    lane.relationship.dirty ? 0 : 1,
    lane.relationship.merged === false ? 0 : 1,
    activeTaskCount(lane) > 0 ? 0 : 1,
    lane.branch || "",
    lane.workspace_path,
    lane.worktree_id,
  ];
}
```

Compare tuple fields in order without locale-dependent mutation. Render the first six lanes and add one `.collapsed-branches` button for the remainder. The button toggles the hidden attribute and its own label.

- [ ] **Step 3: Render each worktree as a branch rail**

Replace `.fork-group`/`.workspace-branch` card rows with one compact `.fork-node` label and one `.branch-rail` per lane. Each lane must contain a `.worktree-stop` button, a horizontal `.rail-path`, `.agent-summary`, progressive `.task-stack`, and the existing explicit `.return-state` text.

Use only `createElement`, `textContent`, `replaceChildren`, class names, `hidden`, `aria-*`, and validated numeric/string data. Do not interpolate model content into HTML or CSS selectors.

- [ ] **Step 4: Implement MAP/READ/FULL**

Add `setDensity(mode)` that accepts only `map`, `read`, or `full`, updates `document.documentElement.dataset.density`, sets exactly one density button to `aria-pressed="true"`, and announces the mode through the existing polite live region. CSS hides `.task-stack` in MAP, shows task title/state but hides `.task-meta` in READ, and shows all task metadata in FULL.

- [ ] **Step 5: Preserve selection and refresh behaviors**

Keep `showRail`, `showStation`, `selectLane`, `showTask`, `showDetails`, hash copy, host-backed refresh, Git-only fallback, SSE/MCP polling, and visibility handling. Update selection styling through `aria-current` without changing the portable model-context payload.

- [ ] **Step 6: Make the graph responsive**

At 620 px and above, use compact grid columns for branch identity, horizontal rail, activity, and return state. Below 620 px, render the same branch DOM as a vertical tree, preserve 44 px coarse-pointer targets, wrap long names, and prevent document-level horizontal overflow.

- [ ] **Step 7: Run the complete Dock UI contract suite**

Run: `cargo test --test dock_ui_contract`

Expected: all Rail View and retained safety/refresh/accessibility contracts pass; `dock_html().len()` remains below 128 KiB.

### Task 3: Browser QA, design comparison, and full verification

**Files:**
- Create: `design-qa.md`
- Modify only if QA finds a defect: `assets/dock.html`, `tests/dock_ui_contract.rs`

**Interfaces:**
- Consumes: the selected source at `.superpowers/brainstorm/product-design/02-rail-view-source.png` and the authenticated local Viewer.
- Produces: browser screenshots at the required breakpoints and a `design-qa.md` whose final line is `final result: passed` or `final result: blocked`.

- [ ] **Step 1: Start the real Viewer and capture the default MAP state**

Run the project Viewer through the existing `devmap dock`/Viewer path. Open it in the Codex in-app Browser, preserve the tokenized URL only inside browser state, and capture 1,024 px MAP mode with real repository data.

- [ ] **Step 2: Exercise core interactions**

At 1,024 px, activate READ and FULL, select a worktree and task, expand/collapse inactive branches when present, invoke refresh, and verify detail/connection labels update. Check console errors after each material state change.

- [ ] **Step 3: Verify responsive states**

Capture 375, 594, 736, and 1,024 CSS px. At every width assert `document.documentElement.scrollWidth <= document.documentElement.clientWidth`, no persistent control is clipped, and long branch/task names remain identifiable.

- [ ] **Step 4: Run Product Design comparison**

Place the selected source and 1,024 px implementation capture into one comparison image. Review typography, spacing, color tokens, graph proportions, copy, and state affordances. Record every P0/P1/P2 issue, fix it with a new failing contract or reproducible browser assertion, recapture, and repeat.

- [ ] **Step 5: Save the blocking QA report**

Write `design-qa.md` with source/implementation paths, viewport and density, interaction and console checks, full-view/focused comparison evidence, iteration history, remaining P3 notes, and exact `final result: passed` only when no actionable P0/P1/P2 findings remain.

- [ ] **Step 6: Run fresh repository verification**

Run:

```text
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets
git diff --check
```

Expected: all commands exit 0, the full suite has zero failures, and only the spec, plan, Dock asset, UI tests, and QA report are modified.

- [ ] **Step 7: Review the branch for handoff**

Run:

```text
git status --short --branch
git diff --stat
git diff --check
```

Do not commit, push, merge, or update the installed plugin until the user asks for that delivery step.
