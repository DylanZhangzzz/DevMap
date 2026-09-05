# DevMap Metro Topology Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver the approved metro view with complete, truthful Git connections, readable workspace/Agent ownership and evidence-backed unfinished-work signals.

**Architecture:** Rust supplies bounded commit/ref topology and independent workspace observations. A small dependency-free JavaScript core projects those facts into stable coordinates and attention states; semantic DOM controls and a shared SVG connection layer render that projection. Preserve the existing host bridges and read-only Git behavior.

**Tech Stack:** Rust 2024 / minimum Rust 1.96, Git CLI, serde, self-contained HTML/CSS/JavaScript, existing tiny_http viewer. Node's built-in test runner for pure JavaScript tests; no new frontend framework or network assets.

**Spec:** `docs/superpowers/specs/2026-09-04-devmap-metro-topology-design.md`

## Global Constraints

- Preserve existing uncommitted work in `assets/dock.html`, `tests/dock_ui_contract.rs`, `design-qa.md` and earlier design documents.
- `devmap/dock/4` producer; v3 input has an explicit limited-view fallback. Invalid snapshots retain the last valid view.
- Preserve the existing 768 KiB Dock model limit and 128 KiB embedded HTML limit.
- Initial topology bounds: 2,048 commits and 256 displayed branch refs; every omitted connection has a boundary or uncertainty label.
- Display names/titles at least 14px and meaningful secondary text at least 12px; no shrink-to-fit canvas.
- Body text contrast at least 4.5:1; essential graphical boundaries at least 3:1.
- Proposed task freshness budget: 120 seconds. Idle attention threshold: 30 minutes. Unknown observations never become definitive inactivity.
- Worktree ID and canonical path establish task ownership. Only a verified Codex thread ID establishes navigation.
- Branch colors never encode risk. Two active tasks are not proof of concurrent writes or a merge conflict.
- Never generate a merge event from `ahead == 0`; that proves ancestry inclusion only.
- Backend collection and UI inspection do not commit, push, merge, reset, fetch or delete user Git state.
- User-selected concept 3 governs the visual language; the spec's explicit corrections govern topology and sample-state correctness.
- Preparation has not changed the installed plugin. Installation/merge/push belong to the later delivery step under applicable user authorization.

## Baseline and file responsibilities

Implementation worktree: `C:/Users/user/Documents/ChatGPT/DevMap-phase-1a-worktree`; branch `codex/rail-view-design-alignment`; inspected HEAD `5741c33` plus pre-existing changes.

Baseline checks: `cargo test --test git_relationship --test dock_model --test dock_ui_contract` — **57 passed**. They do not prove pixel geometry: the inspected running page has a measured 21px station/rail displacement despite these passes.

| File | Responsibility |
|---|---|
| New `src/git_topology.rs` | Bounded commit/ref traversal, graph endpoints and history boundaries |
| Existing `src/git_relationship.rs` | Target resolution and evidence-backed working/integration facts |
| Existing `src/dock.rs` | v4 snapshot, exact task association, freshness and graph budgeting |
| Existing `src/lib.rs` | Export the new topology module |
| New `assets/metro-core.js` | Pure validation, layout, stable colors, attention classification |
| Existing `assets/dock.html` | Semantic rendering, viewport interaction, transport and status feedback |
| Existing `src/dock_asset.rs` | Embed the core into one self-contained HTML asset |
| Existing `src/viewer.rs`, `src/mcp.rs` | Consistent payloads, fresh observations and supported host communication |
| New `tests/git_topology.rs`, `tests/metro_core.cjs` | Semantic graph, risk and coordinate regression tests |
| Existing `tests/dock_model.rs`, `tests/dock_viewer.rs`, `tests/dock_mcp.rs`, `tests/dock_ui_contract.rs` | Compatibility and transport regressions |
| New `tests/fixtures/metro/*.json` | Synthetic scenarios; no user task identities or authentication tokens |

## Shared interfaces

The following names and fields are the contract between tasks; do not independently rename them.

```rust
// src/git_topology.rs; all records derive Debug, Clone, PartialEq, Eq, Serialize.
pub struct TopologyCommit {
    pub oid: String,
    pub parents: Vec<String>,
    pub authored_at: Option<String>,
    pub subject: Option<String>,
}
pub struct TopologyRef {
    pub ref_name: String,
    pub display_name: String,
    pub oid: String,
    pub kind: String, // validated: branch, remote, tag
}
pub struct TopologyEdge {
    pub id: String,
    pub from_oid: String,
    pub to_oid: String,
}
pub struct TopologyBoundary {
    pub id: String,
    pub oid: String,
    pub reason: String, // history_limit, shallow, missing, unrelated
}
pub struct TopologyGraph {
    pub commits: Vec<TopologyCommit>,
    pub refs: Vec<TopologyRef>,
    pub edges: Vec<TopologyEdge>,
    pub boundaries: Vec<TopologyBoundary>,
    pub complete: bool,
}
pub struct GitTopologyCollector;
// Public method:
// GitTopologyCollector::scan(
//   workspace: &SourceWorkspace, worktrees: &[WorktreeDescriptor]
// ) -> Result<TopologyGraph, DevMapError>
```

```javascript
// assets/metro-core.js exports for browser and Node tests:
// validateTopology(graph) -> { valid: boolean, errors: string[] }
// layoutTopology(graph, attachments, { rowGap, columnGap }) -> {
//   nodes: [{ id, x, y, kind }],
//   edges: [{ id, points: [{ x, y }], kind }],
//   attachments: [{ worktree_id, head_oid, x, y }], width, height
// }
// branchColorKey(repositoryId, refName) -> { colorToken, pattern }
// classifyWorkspace(facts, tasks, observation, nowMs) -> {
//   level: 'normal'|'attention'|'unknown', reasons: string[], activeCount: number
// }
// observation: { complete: boolean, observedAtMs: number|null }
// task input: { id, status, lastActivityMs, writeObservedAtMs: number|null }
// facts: { workingState, integration, headRefCoverage, detached, upstream }
```

## Task 1: Collect a real bounded commit/ref graph

**Files:** create `src/git_topology.rs`, `tests/git_topology.rs`; modify `src/lib.rs`. Reuse `tests/support/mod.rs` and its temporary Git fixtures.

**Consumes:** `SourceWorkspace`, `WorktreeDescriptor`, resolved OIDs. **Produces:** `GitTopologyCollector::scan` and the graph types above.

- [ ] Add a real-repository regression proving branches without a worktree are included and every graph edge comes from a commit parent:

```rust
mod support;
use devmap::git::SourceGitInspector;
use devmap::git_topology::GitTopologyCollector;
use devmap::worktrees::WorktreeScanner;

#[test]
fn branch_without_worktree_remains_in_topology() {
    let repo = support::committed_repo();
    support::git(repo.path(), ["branch", "feature/no-worktree"]);
    let workspace = SourceGitInspector::open(repo.path()).unwrap().workspace().unwrap();
    let worktrees = WorktreeScanner::scan(&workspace).unwrap();
    let graph = GitTopologyCollector::scan(&workspace, &worktrees).unwrap();
    assert!(graph.refs.iter().any(|r| r.ref_name == "refs/heads/feature/no-worktree"));
    for edge in &graph.edges {
        let child = graph.commits.iter().find(|n| n.oid == edge.to_oid).unwrap();
        assert!(child.parents.contains(&edge.from_oid));
    }
}
```

- [ ] Run `cargo test --test git_topology` and confirm the missing collector fails before implementation.
- [ ] Implement direct-argument Git reads: enumerate refs with `for-each-ref`; peel tag OIDs; traverse resolved ref tips plus worktree HEADs with bounded `rev-list --topo-order --parents`; enrich the retained commits only. Build edges as actual parent OID → child OID. Parent endpoints outside the retained set receive boundaries.
- [ ] Add fixtures for feature-of-feature, merge commit, fast-forward, multiple refs at one OID, detached unique HEAD, unrelated roots, shallow/truncated history, unborn repository and Git read failure. Use `support::source_snapshot` before/after to assert Git metadata is unchanged.
- [ ] Run `cargo test --test git_topology --test git_relationship`. Review only this task's diff and record its passing tests.

## Task 2: Add truthful workspace facts and v4 freshness

**Files:** modify `src/git_relationship.rs`, `src/dock.rs`, `src/viewer.rs`, `src/mcp.rs`, `tests/dock_model.rs`, `tests/git_relationship.rs`, `tests/dock_viewer.rs`, `tests/dock_mcp.rs`.

**Consumes:** Task 1 graph and existing host task inventory. **Produces:** `devmap/dock/4`, `topology`, per-worktree `workspace_facts`, Git/task observation timestamps, explicit completeness.

- [ ] Add model tests for failed status → `working_state: unknown`, included HEAD + dirty → both facts retained, detached ref coverage, unchanged fresh task observation, omitted inventory versus explicitly empty complete inventory, and byte-limit boundaries.
- [ ] Use this integration distinction in the model; do not infer a merge commit:

```rust
let integration = match (relationship.merge_target.as_ref(), relationship.ahead) {
    (None, _) => "terminal",
    (Some(_), Some(0)) => "included",
    (Some(_), Some(_)) => "ahead",
    (Some(_), None) => "unknown",
};
```

- [ ] Extend status collection to retain success/failure explicitly. The old fallback `dirty: false` is never used to populate `working_state: clean` when Git failed.
- [ ] Preserve actual observation time even when the observed task list is unchanged. Keep structural content hashing stable while fresh envelope timestamps reach clients. Do not confuse task `updated_at` with the time its status was observed.
- [ ] Count unique workspace/task IDs once in v4. Budget topology plus compatibility fields together under 768 KiB; preserve connection boundaries and mark partial rosters. Accept no title/path-based task matching.
- [ ] Update exact schema assertions and run `cargo test --test dock_model --test git_relationship --test dock_viewer --test dock_mcp`.

## Task 3: Validate topology and implement independent attention rules

**Files:** create `assets/metro-core.js`, `tests/metro_core.cjs`, `tests/fixtures/metro/attention.json`; modify `src/dock_asset.rs`, `assets/dock.html` at the core insertion point only.

**Consumes:** Task 2 facts. **Produces:** `validateTopology`, `classifyWorkspace`, `branchColorKey` with the shared interfaces above.

- [ ] Add deterministic tests using a fixed clock. A representative counterexample is:

```javascript
const test = require('node:test');
const assert = require('node:assert/strict');
const { classifyWorkspace } = require('../assets/metro-core.js');
test('included commits do not hide unfinished modifications', () => {
  const facts = { workingState:'dirty', integration:'included', headRefCoverage:'protected', detached:false, upstream:'unknown' };
  const result = classifyWorkspace(facts, [], { complete:true, observedAtMs:2_000_000 }, 2_000_000);
  assert.equal(result.level, 'attention');
  assert.ok(result.reasons.includes('uncommitted_without_active_task'));
});
test('missing roster is not proof of abandonment', () => {
  const facts = { workingState:'dirty', integration:'ahead', headRefCoverage:'protected', detached:false, upstream:'unknown' };
  const result = classifyWorkspace(facts, [], { complete:false, observedAtMs:null }, 2_000_000);
  assert.equal(result.level, 'unknown');
  assert.ok(result.reasons.includes('task_activity_unknown'));
  assert.ok(!result.reasons.includes('uncommitted_without_active_task'));
});
```

- [ ] Run `node --test tests/metro_core.cjs`, then implement all rows of the spec's attention table. Required reason codes include `uncommitted_without_active_task`, `uncommitted_idle`, `not_included`, `unprotected_head`, `shared_workspace`, `concurrent_writes`, `task_activity_unknown`, `git_state_unknown`. Return an array so independent reasons are not overwritten.
- [ ] Validate known schemas, bounded counts, unique OIDs/IDs, valid edge endpoints, actual parent membership, acyclicity and allowed enum values before rendering. Reject malformed input atomically and retain the last good view.
- [ ] Export the core as a browser global and CommonJS module without external dependencies. Embed its source in the HTML via a unique `/* DEVMAP_METRO_CORE */` placeholder in `dock_html()`; keep that function's `&'static str` return using `std::sync::OnceLock<String>`.
- [ ] Run the Node tests and `cargo test --test dock_ui_contract --test dock_viewer`. Preserve the single-asset, no external URL, no browser-storage and injection-defense contracts.

## Task 4: Layout connected rails and workspace attachments

**Files:** extend `assets/metro-core.js`, `tests/metro_core.cjs`; create `tests/fixtures/metro/topology.json` and `tests/fixtures/metro/boundaries.json`.

**Consumes:** validated graph plus `{worktree_id, head_oid}` attachments. **Produces:** `layoutTopology` and stable world coordinates.

- [ ] Encode the spec's synthetic DAG with main/auth/UI/API/experiment/detached paths. Name each edge by its parent and child OID; no edge may end in blank space.
- [ ] Add coordinate invariants, not CSS string assertions:

```javascript
const { layoutTopology } = require('../assets/metro-core.js');
const fixture = require('./fixtures/metro/topology.json');
test('all routed edges terminate on their actual nodes', () => {
  const layout = layoutTopology(fixture.graph, fixture.attachments, {rowGap:96,columnGap:96});
  const nodes = new Map(layout.nodes.map(n => [n.id,n]));
  for (const edge of fixture.graph.edges) {
    const route = layout.edges.find(e => e.id === edge.id);
    const start=nodes.get(edge.from_oid), end=nodes.get(edge.to_oid);
    assert.deepEqual(route.points[0], {x:start.x,y:start.y});
    assert.deepEqual(route.points.at(-1), {x:end.x,y:end.y});
  }
  for (const attachment of layout.attachments) {
    assert.ok(nodes.has(attachment.head_oid));
  }
});
```

- [ ] Implement topological rank, deterministic lanes, station coordinates and edge channels with straight/45-degree turns. State/risk sort cannot reorder graph history. Lane identity and colors remain stable across shuffled input.
- [ ] Preserve branchless commits, common nodes, fork-of-feature, convergence, same-OID workspaces and boundary IDs. Visible edge labels navigate to actual offscreen nodes or load boundaries.
- [ ] Add tests that compare shuffled input, older-history insertion, one task changing state, workspace expansion and at least 50 workspaces. Run `node --test tests/metro_core.cjs`.

## Task 5: Render the selected metro surface at sidebar dimensions

**Files:** modify `assets/dock.html`, `tests/dock_ui_contract.rs`; extend browser geometry scenarios in `tests/fixtures/metro/`.

**Consumes:** Task 4 layout, Task 3 attention results. **Produces:** connected rails, readable semantic stations/tasks, compact header and independent warnings.

- [ ] Replace the separate CSS-grid line/station positioning with one positioned world containing an SVG connection layer and semantic controls at the same coordinates. Node hit areas may be larger than dots without moving their centers.
- [ ] Use named branch and semantic-status tokens. Keep branch strokes stable across status changes; use neutral text for pale/yellow branches. Implement the specified text sizes and contrast; correct the self-targeting `.shell` container query.
- [ ] Retain the exact visible labels `DevMap · Rail View` and `Repository topology`; update the document title consistently. Keep current-workspace identification and task names prominent; hide the empty inspector until selection.
- [ ] Default to a compact branch label column and a visible current workspace. At narrow widths allocate width to the graph, then reveal details on selection; never reduce text to fit all history.
- [ ] Update obsolete source-string tests intentionally. Keep tests for escaping, host transport, bounds and identities, but verify geometry in the browser with this invariant:

```javascript
// Run inside the supported browser's read-only evaluate scope after rendering.
const nodes=[...document.querySelectorAll('[data-commit-oid]')];
const failures=nodes.filter(node=>{
  const r=node.getBoundingClientRect();
  const dot=node.querySelector('[data-station-dot]');
  if(!dot) return true;
  const d=dot.getBoundingClientRect();
  return Math.abs((d.x+d.width/2)-(r.x+r.width/2))>1
    || Math.abs((d.y+d.height/2)-(r.y+r.height/2))>1;
});
// Pair this with layout-edge endpoint checks from Task 4 and actual SVG positions.
```

- [ ] Capture actual 360/480/526/900/1440px states in one batch and inspect task-title visibility, branch endpoints, line crossings, focus, hit areas and 200% text scaling. Baseline defects are recorded in the audit; a passing Rust contract does not close them.

## Task 6: Preserve exploration state and task navigation

**Files:** modify `assets/dock.html`, `src/mcp.rs`, `src/viewer.rs`, `plugins/devmap/skills/live-worktree-dock/SKILL.md` only where host capability evidence supports a change; extend `tests/dock_mcp.rs`, `tests/dock_viewer.rs`, `tests/metro_core.cjs`.

**Consumes:** verified task IDs and workspace IDs. **Produces:** usable task drill-down, host navigation, local viewport state and separate status feedback.

- [ ] Store `selectedWorkspaceId`, `selectedTaskId`, expanded workspace/history IDs and viewport position independently of DOM nodes. Refreshes reconcile these IDs; if an object disappears, show a specific unavailable state.
- [ ] Keep all active tasks discoverable. Default summary previews two active names, then an explicit active count. Expanding history never covers rails; expansion and collapse preserve keyboard focus.
- [ ] Implement background drag, native horizontal scroll, Shift+wheel, arrows/Home/End and a labeled pan control. Clickable Agent rows must not start dragging. Current/risk locating must move the appropriate station into view.
- [ ] Split connection freshness, task inventory freshness and action feedback into separate elements. Git heartbeat cannot erase a navigation failure or a refresh explanation. Derive activity age from the correct observation source.
- [ ] Verify actual available host navigation before changing it. Preserve validated-ID-only requests, pending/error timeout and inspector fallback. Do not label unsupported links as working jumps. No user task title or path may become an executable host instruction.
- [ ] Exercise real navigation to one existing task and return, plus unsupported/timeout paths using fixture capabilities. Record the observed destination task ID, not just “Opening…”. Verify task inventory refresh changes observation freshness even when names/statuses are unchanged.

## Task 7: Run focused semantic, visual and Impeccable acceptance

**Files:** existing/new test files, updated `design-qa.md`, a new audit report and screenshot evidence.

**Consumes:** the complete candidate and synthetic fixtures. **Produces:** an evidence-backed release decision with explicit open items.

- [ ] Run `cargo fmt --check`, focused Rust tests for topology/model/viewer/MCP/UI contracts, and `node --test tests/metro_core.cjs`. Run the existing relevant long-session performance scenario after graph cache/budget changes; do not add unbounded repeated graph walks per heartbeat.
- [ ] Verify every scenario in the spec's coverage matrix. Record semantic and visual results separately; an unavailable bridge or missing external observation remains unverified.
- [ ] Use Impeccable's Operate guidance and `reference/craft-floor.md` during implementation. After the candidate is complete, run `impeccable.cmd detect --json assets/dock.html` once against the assembled page as needed, review computed styles and document rejected false positives.
- [ ] Take one batched screenshot round covering narrow/actual/full widths plus interaction states; apply material fixes in one batch and confirm once. Compare the revised graph against both concept 3 and the exact synthetic graph; do not reproduce concept-image defects for pixel similarity.
- [ ] Obtain the Impeccable finish review with the spec, source, screenshots, detector findings and explicit topology invariants. Close material findings; record final built tokens/components in `DESIGN.md` only after implementation. Do not write planned styling as if it were the current implementation.

## Task 8: Verify the delivery artifact and installed runtime

**Files:** existing build/install scripts and release evidence; source changes only if the verified packaging path needs them.

**Consumes:** accepted candidate; applicable delivery authorization. **Produces:** provenance from tested source to the opened runtime.

- [ ] Record source revision plus dirty-diff identity, assembled HTML SHA-256, schema version and executable SHA-256. Avoid a version label that conceals uncommitted source changes.
- [ ] Build once using the repository's existing release/install workflow. At the time of this preparation, the user's open Dock runs a temporary debug executable; this is not evidence of the installed skill version.
- [ ] When installation is in scope, verify the destination versioned executable, the actual running process path and the served HTML/script fingerprint. Restart only the identified DevMap process as required; preserve source Git state.
- [ ] Open the delivered Dock using its current skill and host task inventory requirements. Check header/title, topology, task navigation and attention cases again from that runtime. An app tool returning `queued` is not proof the panel appeared.
- [ ] Stage/commit/push/merge only the intended completed change when that delivery step is authorized. Report actual verification and remaining limitations, not a completion percentage.

## Preparation handoff

The user has already chosen the visual direction and requested development preparation. No additional palette/option selection is needed. Implement Task 1 first; it is the prerequisite that prevents disconnected lower rails from recurring. This plan's execution boxes remain unchecked because this turn prepares and audits the work rather than declaring the new renderer implemented.
