# DevMap Branch Topology and Task Refresh Design

**Date:** 2026-09-03

**Status:** Approved

**Supersedes:** the flat per-worktree relationship layout in `2026-09-02-devmap-live-worktree-dock-design.md`

## 1. Goal

Make the DevMap Dock answer these questions in one glance:

- Which Codex task is working in each local Git workspace?
- Which branch is checked out in that workspace?
- Which integration branch should receive that branch (`dev`, `develop`, or `main`)?
- At which exact commit does the branch diverge from its integration target?
- Which worktrees share the same branch point?
- Has each branch merged back into its immediate integration target?
- Has the development branch itself merged back into `main`?

The map must also update a Codex task title after the task is renamed. It must not read Codex private databases or imply Git knows the historical branch-creation command when that information is not recorded.

## 2. User-visible model

The selected layout is a simplified hierarchical branch map.

```text
main history
  |
  o common base with main
  +-- dev workspace / tasks ---------------- not merged -> main
       |
       o common base with dev
       +-- dylan_test workspace / tasks ----- not merged -> dev
       +-- Joe_dev workspace / tasks -------- merged ----> dev
```

The map shows one rail per integration branch and one station per exact common-base commit. Worktrees whose branches have the same integration target and the same common-base hash fan out from the same station. A different common-base hash produces another station on the same target rail.

The default graph contains only:

- integration branch rails and names;
- common-base stations with short hashes and worktree counts;
- branch names;
- short workspace names;
- Codex task titles and active/idle state;
- visible merge result text and colored edges.

Full paths, full HEAD hashes, full common-base hashes, exact tags, commit subject/date, ahead/behind counts, dirty-file counts, capture warnings, and association provenance appear only in the selected-node detail region. This progressive disclosure prevents the graph from becoming another dense table.

## 3. Git semantics

### 3.1 Integration hierarchy

DevMap constructs a bounded integration hierarchy rather than guessing historical parent branches.

1. Resolve the root integration branch from the remote default branch, then local `main`, then local `master`.
2. Resolve the development integration branch from configured `devmap.developmentTarget`, then local `dev`, then local `develop`.
3. If a distinct development branch exists, ordinary worktree branches compare and return to it; the development branch compares and returns to the root branch.
4. If no distinct development branch exists, ordinary branches compare and return directly to the root branch.
5. The root branch is the terminal rail and has no return edge.

Configuration remains authoritative only when the referenced branch exists and passes the existing bounded ref validation. The UI calls each station `Common base with <target>` or `Diverges from <target> at`; it does not claim Git recorded a historical `git branch` origin.

### 3.2 Exact branch points

For each unique `(target ref, worktree HEAD)` pair, run:

```text
git merge-base <target-ref> <worktree-head>
```

The resulting full object ID is the exact common-base hash used for grouping. DevMap also reads:

- exact tags from `git tag --points-at <common-base>`;
- the bounded subject and authored date from `git show -s`;
- ahead/behind counts from `git rev-list --left-right --count`;
- merge state from reachability (`ahead == 0` relative to the immediate target);
- dirty state from the existing porcelain status parser.

Tags are shown only when they point exactly at the common-base commit. Tag names do not affect grouping. Worktrees are never grouped by short hash, path similarity, branch-name prefix, timestamps, or visual proximity.

### 3.3 Ambiguity and failures

Git does not retain the command that originally created a branch. The integration hierarchy describes the current comparison and merge targets, while the common-base hash describes exact ancestry.

If a target ref or merge-base is unavailable, the affected branch enters an `Unknown base` group. The map shows a dashed edge and an integrity warning. DevMap does not attach it to the nearest-looking station.

If two configured target candidates would make the hierarchy ambiguous, configuration wins; otherwise DevMap uses the deterministic precedence above and exposes the selected target source.

## 4. Read model

The Dock schema advances to `devmap/dock/2`. Compatibility arrays used by text output may remain during this change, but the frontend consumes the hierarchy exclusively.

Conceptual types:

```rust
struct IntegrationBranch {
    name: String,
    ref_name: String,
    head: String,
    parent: Option<String>,
    source: TargetSource,
}

struct ForkPoint {
    target_branch: String,
    commit: String,
    tags: Vec<String>,
    subject: Option<String>,
    authored_at: Option<String>,
    distance_to_target: Option<u32>,
}

struct BranchGroup {
    fork_point: Option<ForkPoint>,
    lanes: Vec<DockLane>,
}

struct DockReadModel {
    integration_branches: Vec<IntegrationBranch>,
    branch_groups: Vec<BranchGroup>,
    task_inventory_synced_at: Option<String>,
    // revision, warnings, truncation, compatibility fields
}
```

All new strings and arrays inherit explicit bounds. Tags are sorted and capped. Commit subjects are display text, not instructions. The content hash covers the integration hierarchy, fork metadata, lane membership, task titles/status, and task-inventory sync timestamp only when the supplied inventory changes. Polling unchanged Git state must not increment the revision.

## 5. Codex task-title refresh

### 5.1 Root cause

The current browser Refresh button asks DevMap to recompute Git and Presence state but reuses the `codex_tasks` inventory previously supplied by the plugin. Renaming a Codex task changes the host task list, not DevMap's retained inventory, so the old title remains until another host-side `list_threads` call supplies a replacement.

### 5.2 Refresh flow

DevMap keeps host data acquisition outside the Rust server:

1. The user clicks `Refresh all` in the Dock.
2. In an MCP App, the frontend sends a portable `ui/message`; in a Codex integrated Browser, it uses the documented host follow-up-message bridge when available.
3. The message asks the current Codex task to refresh DevMap. It contains no task titles, paths, hashes, tokens, or other private state.
4. The DevMap skill calls `list_threads`, keeps local Codex tasks with supported states, copies only the approved bounded fields, and calls the Dock snapshot/start tool with a complete `codex_tasks` replacement.
5. The MCP runtime replaces—not appends to—the retained inventory and updates the running Viewer state.
6. The Viewer stream or MCP tool result publishes the new revision. Matching session IDs retain identity while `display_title` and host status change.

The button shows `Requesting Codex…` immediately, remains disabled during the request, and returns to `Refresh all` after a newer task-inventory sync marker arrives. A successful title refresh is announced through one polite live region.

### 5.3 Fallback behavior

Host follow-up messaging is capability-gated. If unavailable:

- the button performs the existing Git-only snapshot refresh;
- the connection label says `Git refreshed · task names last synced <age>`;
- the UI provides a visible instruction to ask Codex `Refresh DevMap`;
- the page never claims task names are current.

The design deliberately rejects reading Codex SQLite files, scanning session logs, injecting into the Codex DOM, or adding a local endpoint that accepts untrusted task inventory from arbitrary browser JavaScript.

## 6. Interaction design

### 6.1 Selection and details

Common-base stations, branch/workspace rows, and task rows use native buttons with visible focus styles. Selecting a node updates one compact detail region below the graph; no persistent card column is added.

Selecting a common-base station shows:

- integration target name;
- full common-base hash;
- exact tags or `No exact tag`;
- bounded commit subject and authored date;
- number of attached branches/worktrees;
- a `Copy hash` button.

Selecting a branch/workspace shows the full branch name, full path, full HEAD, immediate merge target, common-base hash, ahead/behind counts, merge state, dirty count, and attached tasks. Selecting a task shows its full title, exact local session ID, workspace association source, and host status.

Clipboard success changes the control label to `Copied` and announces success. Failure says `Copy unavailable` and leaves the full hash selectable. Hash copying is a local user-initiated action.

### 6.2 Visual hierarchy

- blue: root integration rail (`main`/`master`);
- green: development integration rail (`dev`/`develop`);
- purple: ordinary branch/task identity;
- green edge plus `Merged`: reachable from the immediate target;
- amber edge plus `Not merged`: commits remain ahead of the immediate target;
- neutral dashed edge plus `Unknown`: relationship could not be proven;
- red dot plus text: dirty workspace, independent of merge state.

Color is never the only state signal. Structural rails use restrained neutral/semantic strokes; repeated nodes do not receive decorative cards or shadows.

### 6.3 Responsive behavior

At widths of 620 px and above, the map shows integration rails, branch rows, and return edges in a compact horizontal composition. At narrower widths, the same DOM order becomes a vertical tree:

```text
target rail
  common-base station
    branch/workspace
      tasks
      merge result
```

There is no document-level horizontal scrolling, clipped target node, fixed viewport height, or nested full-height scroller. Text wraps; essential branch and task names are not ellipsized without an accessible full value. Controls remain at least 44 px on coarse pointers and preserve native keyboard order.

## 7. Data flow and component boundaries

```text
Git refs/worktrees ──> GitTopologyResolver ──> integration hierarchy + fork groups
Presence/journals ───> DockReducer ──────────> workspace/task operational state
Codex list_threads ──> MCP argument parser ──> replaceable observed-task inventory
                                                   |
                                                   v
                                      revisioned DockReadModel v2
                                         |                 |
                                   MCP App bridge     Viewer HTTP/SSE
                                         |                 |
                                         +------ UI -------+
```

`GitTopologyResolver` owns exact Git commands and topology facts. `DockReducer` joins topology, Presence, journals, and observed tasks without shelling out. `McpRuntime` validates and replaces the host task inventory. `ViewerRuntime` shares the same in-process service state. The frontend renders only the bounded read model and never reconstructs ancestry from names.

## 8. Testing strategy

Implementation follows red-green-refactor. Required tests include:

1. Two feature branches with the same target and common-base hash produce one fork group with two lanes.
2. Branches with different common-base hashes produce separate, correctly ordered stations.
3. With `main`, `dev`, `dylan_test`, and `Joe_dev`, features target `dev` and `dev` targets `main`.
4. A tag appears only when it points exactly at the common-base commit.
5. Full hashes, bounded subject/date, ahead/behind, and merge state serialize correctly.
6. A merge-base failure produces `Unknown base` and a warning without dropping the workspace.
7. Supplying the same session ID with a renamed title replaces the title and advances the model revision.
8. Omitting `codex_tasks` retains the last inventory; supplying an empty array clears it.
9. Remote tasks, unsupported states, unrelated paths, and oversized fields remain excluded or rejected as currently specified.
10. Frontend contract tests require one integration rail per branch, one station per fork group, progressive details, copy feedback, visible state text, and safe `textContent` rendering.
11. Browser QA covers 375, 594, 736, and 1,024 px, keyboard focus, task-title refresh feedback, copy feedback, no horizontal overflow, and no console errors.
12. Full formatting, Clippy, Cargo tests, privacy canary checks, plugin validation, cachebuster installation, and real Codex reopen/refresh validation pass before completion.

## 9. Delivery boundaries

This change covers local worktrees sharing one Git common directory and local Codex task inventory supplied by the host. It does not add cross-machine monitoring, infer organization ownership, navigate automatically to arbitrary tasks, mutate branches, merge code, or persist personal graph layout.

The updated plugin is installed through the cachebuster workflow. Because Codex loads plugin capabilities per task, final end-to-end validation uses a fresh Codex task after installation.
