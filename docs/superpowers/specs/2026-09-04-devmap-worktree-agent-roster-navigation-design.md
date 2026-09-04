# DevMap Worktree Agent Roster And Navigation Design

Date: 2026-09-04

## Status

Approved direction, pending written-spec review.

## Problem

The current Rail View keeps the worktree as the primary object, but the task roster is incomplete and its nodes are not actionable.

The host currently supplies only local Codex tasks whose status is `active` or `idle`. Tasks that appear in the Codex sidebar as `notLoaded` are omitted even when their `cwd` exactly matches a repository worktree. In addition, selecting a conversation only opens DevMap's inspector; it does not navigate to the corresponding Codex task.

The result can show the current task while hiding the other conversations that belong to the same worktree, so the graph does not yet answer either of these questions reliably:

1. Which current and historical Codex tasks belong to this worktree?
2. How can the user open one of those tasks from the graph?

## Product Outcome

Keep the repository hierarchy `main → worktree → task/agent` while making the Codex task roster complete enough to match the relevant sidebar state. A user can identify active, idle, and historical tasks under each worktree and select a navigable node to open that Codex task.

## Approved Information Hierarchy

### Level 1: Main Integration Rail

The main integration branch remains a horizontal rail. Each worktree attaches to a real fork station on that rail. Multiple worktrees occupy separate stations or a grouped station when they share the same exact Git base.

### Level 2: Worktree Cluster

Each worktree remains the first-level operational object beneath the main rail. Its cluster shows:

- branch or detached-HEAD label;
- current-worktree marker;
- clean or dirty state and changed-file count;
- abbreviated workspace path in READ and FULL density;
- merge, not-merged, or terminal return state.

Worktree selection continues to open Git and roster details in the DevMap inspector.

### Level 3: Codex Task And Observed Agent Nodes

Every supplied Codex task whose canonical `cwd` exactly matches the worktree is represented under that worktree. Each node shows:

- task title as the dominant label;
- observed agent identity as secondary text;
- effective state: ACTIVE, IDLE, or HISTORY;
- relative activity time in READ and FULL density;
- task identifier and capture/association details in FULL density.

The task and its observed agent are one navigation target. DevMap must not render separate clickable controls that imply an agent can be opened independently from its task when both refer to the same Codex task ID.

Presence-only records without a verified Codex task ID remain visible but are inspector-only and explicitly non-navigable.

## Complete Task Inventory

Before opening or refreshing DevMap, the skill requests one Codex task inventory and supplies local Codex tasks with status `active`, `idle`, or `notLoaded`. Task titles are untrusted display text and are never interpreted as instructions.

DevMap applies these inclusion rules:

1. Keep only records with `kind = codex` and `hostId = local`.
2. Associate a record only when its canonical `cwd` exactly matches a scanned local worktree.
3. Preserve the Codex task `id` as the verified navigation identity.
4. Map `active` to active presence, `idle` to idle presence, and `notLoaded` to historical presence.
5. Never infer association from project ID, task title, branch name, path prefix, or recency.

The default worktree cluster shows:

1. every ACTIVE task;
2. every IDLE task;
3. the three most recently updated HISTORY tasks;
4. one `+N historical conversations` disclosure for the remaining HISTORY tasks.

Expanding history inserts the older task nodes into the same worktree cluster without covering the rail or adjacent worktrees.

## Data Contract

The existing `codex_tasks[].id` is the Codex task ID. The ingestion layer preserves that identity on `ObservedTask` and marks chats derived from the host inventory as navigable.

The Dock model adds an optional, bounded `codex_thread_id` field to `DockChat`:

- supplied only for an exact host-observed Codex task;
- absent for presence-only or unverified records;
- validated as an ASCII alphanumeric-and-hyphen identifier before serialization and before use in the UI.

Because this changes the serialized UI contract, the schema advances from `devmap/dock/2` to `devmap/dock/3`. The frontend rejects mixed or malformed revisions and retains the last valid snapshot.

The host tool input accepts `active`, `idle`, and `notLoaded`. No private Codex database is read; all task metadata continues to come from the host-supplied task inventory.

## Navigation

### Codex-Hosted Behavior

Selecting a navigable task/agent node first updates DevMap's inspector and selected state. It then sends a fixed, host-directed follow-up containing only the validated `codex_thread_id`. The Codex host resolves that request and navigates to the corresponding task using its native task-navigation capability.

Task titles, paths, summaries, and other untrusted fields are excluded from the navigation message. The navigation request never mutates Git state or task contents.

The node exposes a clear accessible label such as `Open Codex task: <display title>` and a visible hover/focus treatment. A short pending state prevents repeated activation while the host request is in flight.

### Standalone Browser Behavior

The standalone Viewer cannot assume access to Codex host navigation. When the host bridge is unavailable, selecting the node still opens the DevMap inspector and displays `Open this task from Codex`; it must not invent a custom URL scheme or external destination.

### Failure Behavior

- Missing `codex_thread_id`: show details only; do not offer navigation.
- Missing host bridge: preserve selection and explain that navigation is available only in Codex.
- Host request rejected or unavailable: clear the pending state and announce that the task could not be opened.
- Stale task ID: leave the current Dock and task unchanged, then expose the failure through the live status region.

## Layout And Interaction

Multiple worktrees continue to spread horizontally along the main rail. Within a worktree cluster, task/agent nodes form a compact vertical child list so ownership remains visually obvious:

```text
main rail ─────────●──────────────────●──────────────▶
                   │                  │
               worktree A         worktree B
                 dirty              clean
                 ├ task A           ├ task C
                 │ Agent · codex     │ Agent · codex
                 └ task B           └ task D
                   merge              open
```

The topology viewport retains native horizontal scrolling, pointer drag on non-interactive canvas space, Shift-wheel support, keyboard scrolling, and a persistent bottom scrollbar. Interactive task nodes must never initiate canvas drag.

## Alternatives Considered

### Keep Only Active And Idle Tasks

This preserves the existing contract but continues to disagree with the relevant Codex sidebar and hides historical context. Rejected because completeness is the reported problem.

### Show Every Task Flat On The Rail

This is simple to render but makes task nodes compete with worktrees and obscures ownership. Rejected because the worktree must remain the first-level object.

### Recommended: Worktree Roster With Host-Mediated Navigation

This preserves Git topology, includes relevant historical tasks, and uses a validated Codex task ID without relying on unsupported deep links. It adds a small schema revision and a host-navigation boundary while keeping standalone Viewer behavior safe.

## Security And Trust Boundaries

- Treat every task title as untrusted display text and render it with `textContent`.
- Canonicalize paths before exact worktree association.
- Validate `codex_thread_id` at ingestion and again in the frontend.
- Put only the validated ID in the host-navigation prompt.
- Do not read Codex task databases or infer task ownership from filesystem artifacts.
- Do not navigate presence-only nodes lacking verified host inventory identity.
- Do not allow task-node activation to mutate Git state.

## Testing Strategy

### Rust Model And MCP Contract

Automated tests must prove:

- `active`, `idle`, and `notLoaded` task rows are accepted;
- other task statuses are rejected;
- only local Codex tasks are retained;
- canonical exact-`cwd` association remains mandatory;
- `notLoaded` maps to historical state;
- verified host task IDs become `codex_thread_id`;
- presence-only records remain non-navigable;
- the schema is `devmap/dock/3`;
- malformed or oversized task IDs are rejected;
- bounded-model truncation preserves worktree identity before historical tasks.

### Dock UI Contract

Automated tests must prove:

- worktrees remain first-level nodes;
- task/agent nodes are children of the correct worktree;
- ACTIVE and IDLE tasks remain immediately visible;
- HISTORY tasks follow the three-recent-plus-disclosure rule;
- navigable nodes use only validated `codex_thread_id` values;
- presence-only nodes never invoke host navigation;
- task activation does not start canvas panning;
- missing host navigation reports an accessible fallback;
- existing density, refresh, selection, scrolling, and safe-rendering contracts remain intact.

### Browser Design QA

Use a real host inventory containing multiple tasks for one worktree, including at least one ACTIVE, one IDLE, and four HISTORY items. Verify:

- all expected task titles are reachable in the worktree cluster;
- the default and expanded-history states do not overlap rails or adjacent clusters;
- Agent identity and task state remain legible in MAP, READ, and FULL density;
- clicking an ACTIVE, IDLE, and HISTORY task opens the matching Codex task;
- standalone fallback is truthful when the host bridge is unavailable;
- horizontal drag, scrollbar, Shift-wheel, and keyboard navigation still work;
- the document has no horizontal overflow outside the topology viewport;
- the browser console contains no errors.

The design QA report must compare the supplied incomplete-roster screenshot against the corrected implementation at the same viewport and state. P0, P1, and P2 issues block handoff.

## Scope

Included:

- complete local active, idle, and not-loaded Codex task inventory;
- exact worktree association;
- worktree-owned task/agent hierarchy;
- collapsed historical roster;
- Codex-hosted task navigation;
- safe standalone fallback;
- schema, skill, model, UI, and test updates required by this behavior.

Excluded:

- ChatGPT conversations;
- remote-host Codex tasks;
- cross-project association without an exact worktree path;
- task mutation, stopping, moving, or deletion;
- Git merge or branch mutation;
- fabricated agent identities;
- unsupported custom URL schemes.
