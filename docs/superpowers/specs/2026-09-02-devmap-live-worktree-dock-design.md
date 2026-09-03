# DevMap Live Worktree Dock Design

**Date:** 2026-09-02 (amended 2026-09-03)

**Status:** Approved

**Related requirements:** `docs/ai-development-map-requirements.md` sections 16.8, 16.9, and 21.4

**Dependencies:** Phase 1B capture journals; Phase 1C route registry is an optional enrichment, not an MVP prerequisite

## 1. Goal

Add a compact, live view of the current worktree and other local worktrees to the right side of the current development chat. The Dock answers:

- Which worktree am I in?
- Which Codex chat or Agent is associated with each local worktree, and which chats are unlinked or uninstrumented?
- What branch, HEAD, route, and capture state does each one have?
- Which development target (`dev`, `develop`, or the remote default branch such as `main`) does that branch belong to, and has it been merged back?
- Is an Agent working, waiting, idle, completed, stale, or simply unknown?
- Where should I click to inspect that Agent's route and evidence neighborhood?

The Dock is an operational overlay on the canonical development map. It is not a second map, a member-specific shared view, or evidence by itself.

## 2. Product placement

The preferred Codex layout is:

```text
┌────────────────┬──────────────────────────┬──────────────────────┐
│ Project sidebar│ Current development chat │ DevMap Live Dock     │
│                │                          │ Current Worktree     │
│                │                          │ Other Worktrees      │
└────────────────┴──────────────────────────┴──────────────────────┘
```

This is a collapsible right pane inside the same Codex window. It must not cover the message transcript. The host controls whether an MCP App renders inline or in a pane and whether any pane remains pinned across chat changes or application restarts. DevMap must not claim persistence or placement the host does not expose.

Presentation order:

1. MCP App in the host-selected embedded location when the host supports it. After the DevMap plugin is installed and enabled, Codex starts the plugin's `devmap mcp` STDIO process; the user does not start a local HTTP server.
2. The same bundled frontend in Codex's integrated Browser panel. An explicit right-side open or reopen request lazily starts a Viewer owned by the MCP process and asks Codex to place its localhost page in the right Browser pane.
3. The same authenticated localhost URL in an external browser, started on demand or explicitly with `devmap view --live`.

The official OpenAI documentation defines a configured `command` as the launcher for a local STDIO MCP server, supports optional UI returned by MCP servers, and supports opening localhost applications in the integrated Browser. It does not expose a plugin manifest contribution for a permanent custom right-sidebar item. DevMap must use documented host surfaces, must not inject into the Codex DOM or call private UI APIs, and must describe the Browser pane as a reopenable fallback rather than a host-persistent native Dock.

## 3. Considered approaches

### 3.1 Host-native MCP App plus shared web fallback — selected

Expose a small MCP App resource for hosts that support embedded UI. Its registered plugin configuration starts `devmap mcp` over STDIO and uses the shared Dock read model directly. Reuse the same frontend through an on-demand Viewer elsewhere. For Codex, a read-only `devmap_start_browser_dock` MCP tool lazily starts or reuses that Viewer and returns its authenticated loopback URL; the plugin skill then asks the documented Codex Browser surface to open the URL on the right.

Advantages:

- matches the requested chat-right placement;
- requires no manual server command in the normal Codex path;
- keeps DevMap host-neutral;
- shares one frontend and one read-model contract without forcing both surfaces through HTTP;
- degrades without losing core functionality.

Cost: the host capability handshake, Viewer lifecycle, URL handoff, and browser fallback all require conformance tests.

### 3.2 Localhost Browser panel only

This is simpler and already supported for local applications, but it cannot promise that every host treats the page as a persistent chat-side tool. It remains the mandatory fallback.

### 3.3 Native always-on-top desktop window

This works across editors but adds packaging, window lifecycle, signing, and accessibility work. It is deferred until users demonstrate a need to keep the Dock visible outside the development host.

## 4. Architecture

```text
Codex / Claude / Generic MCP adapters
              │ bounded lifecycle and activity facts
              ▼
Per-worktree Phase 1B journals
              │
              ├──────────────┐
              ▼              ▼
Git worktree scanner   Presence lease store
              │              │
              └──────┬───────┘
                     ▼
            Local Presence Reducer
                     │
                     ▼
       revisioned `DockReadModel`
              │              │
              │              └── temporary Viewer (on demand)
              │                         │ loopback HTTP + SSE
              ▼                         ▼
 `devmap mcp` STDIO bridge        Browser fallback
  (Codex-managed lifecycle)
              │
              ▼
    host-selected MCP App surface
```

The architecture has six isolated components:

1. **Worktree scanner:** enumerates `git worktree list --porcelain` and resolves a stable worktree identity from the Git directory, never from a renameable display path alone.
2. **Presence writer:** updates one bounded atomic record for an instrumented session when an accepted Hook or MCP lifecycle event occurs.
3. **Presence reducer:** joins worktrees, local presence, Phase 1B journals, and available Phase 1C route records into a revisioned `DockReadModel`.
4. **MCP bridge:** exposes the read model and bundled UI through `devmap mcp` over STDIO. Codex owns process startup and shutdown through the installed plugin configuration. Snapshot and embedded-App calls open no TCP listener.
5. **Browser bridge:** when explicitly requested, `devmap_start_browser_dock` lazily starts or reuses a Viewer owned by the MCP runtime and returns its authenticated URL. The explicit CLI command `devmap view --live` serves the same frontend for non-MCP use. Both paths provide an initial snapshot and monotonic in-process deltas over loopback SSE and expose no mutation endpoint.
6. **Dock frontend:** renders the same read-model contract in either presentation and asks the shared Viewer to focus a selected worktree or route.

The compact Viewer is a Git relationship map, not a dashboard of independent cards. Each lane reads left to right:

```text
base target ── branch + workspace ── associated chat / Agent ── merge target
    main          codex/example          "Implement X"             main
      └───────────────────────────────────────────────────────────► merged
```

The left target shows where the work diverged; the right target shows whether it returned. A merged branch uses a continuous green return edge into the right-side target. An unmerged branch uses an amber edge that stops before the target and shows exact ahead/behind counts. Chat or Agent nodes attach to the workspace lane rather than sitting in a separate list.

These components share schema types but not implementation internals. The Browser and MCP App presentations consume the same `DockReadModel`, but each uses its host-appropriate transport. HTTP is not an internal dependency of the MCP path.

## 5. Storage and authority

### 5.1 Ephemeral Presence

Repository-wide local Presence lives under the resolved Git common directory:

```text
<git-common-dir>/devmap/presence/v1/<session-id>.json
```

All linked worktrees can read this directory. It is neither a custom ref nor a committed file. Writes use canonical JSON, bounded fields, atomic replacement, no-follow path checks, and the existing DevMap coordination primitives.

Presence records are derived and disposable. They must never be referenced as proof that code, tests, review, or a deployment succeeded. Canonical journals and Context objects remain the evidence sources.

### 5.2 Shared and personal state

- Project graph objects and the authorized global topology layout remain shared through the Context Repository.
- Presence is local runtime state and may differ across machines.
- Camera, zoom, hover, selection, scroll, and temporary filters exist only in the browser session.
- Heartbeats and UI state never cause a source or Context commit.

This separation preserves the requirement that every member sees the same graph for the same graph revision while allowing each machine to overlay its own live processes.

## 6. Presence schema

A reduced record contains only operational identifiers and summaries:

```json
{
  "schema_version": 1,
  "repository_id": "sha256-...",
  "worktree_id": "wt-...",
  "session_id": "session-...",
  "actor_id": "codex:session-...",
  "host": "codex",
  "route_id": "route-...",
  "branch": "codex/example",
  "head": "0123456789abcdef0123456789abcdef01234567",
  "status": "working",
  "status_source": "capture_event",
  "confidence": "observed",
  "capture_grade": "D",
  "last_event_at": "2026-09-02T12:00:00Z",
  "lease_expires_at": "2026-09-02T12:02:00Z",
  "current_activity_id": "activity-...",
  "current_decision_id": null,
  "blocker_count": 0,
  "gap_count": 1
}
```

Every string, array, record, directory, and aggregate response has an explicit size or count limit. The record excludes prompts, commands, patches, tool arguments, tool results, file contents, and transcript text.

The local API may add display-only fields such as a shortened path or branch label. Those fields are not written into canonical evidence.

### 6.1 Git relationship projection

Each worktree row adds a bounded, derived relationship projection:

```json
{
  "workspace_path": ".worktrees/devmap-live-worktree-dock",
  "branch": "codex/devmap-live-worktree-dock",
  "dirty": true,
  "changed_file_count": 2,
  "base_target": "main",
  "merge_target": "main",
  "merged": false,
  "ahead": 35,
  "behind": 0,
  "chat": {
    "session_id": "01a0...",
    "title": "Open DevMap Dock",
    "status": "working",
    "association_source": "host_explicit"
  }
}
```

Merge state is recomputed from Git ancestry. DevMap selects the repository's configured development target when available, otherwise a local `dev` or `develop`, otherwise the remote default branch, and finally local `main` or `master`. The chosen ref and source are displayed. DevMap never treats equal branch names, zero file changes, or a closed chat as proof of merge.

A chat is attached only through an exact host session/worktree identifier or an explicit adapter association. A matching path without a matching session is not enough to guess ownership. Missing data is labeled `No linked chat` or `Uninstrumented`; `Ignored` is reserved for an explicit host/user ignore decision.

## 7. Status semantics

Allowed UI states are:

| State | Required basis |
| --- | --- |
| `starting` | accepted SessionStart without later activity |
| `working` | explicit host-running signal or recent accepted activity with a valid lease |
| `waiting` | explicit normalized approval/input-waiting signal only |
| `idle` | TurnCompleted while the session remains open |
| `completed` | explicit SessionEnd only |
| `stale` | an open session whose lease expired without SessionEnd |
| `unknown` | worktree exists but no trustworthy instrumented session state exists |

`stale` must never be relabeled `completed`. A missing record does not prove that no Agent exists. When a host does not expose reliable running or waiting signals, the reducer lowers `confidence` instead of guessing.

`status_source` is one of `host_explicit`, `capture_event`, `lease`, or `git_only`. `confidence` is one of `observed`, `leased`, `inferred`, or `unknown`. The UI shows uncertainty directly.

## 8. Presentation transports and lifecycle

### 8.1 Default Codex MCP path

The DevMap plugin registers `devmap mcp` as a local STDIO MCP server. After the plugin is installed and enabled, Codex launches the configured command and manages its lifecycle. The user must not need to run `devmap view`, `devmap view --live`, or a separate daemon to open or refresh the right-pane Dock.

The MCP server returns the bundled Dock UI and a read-only, revisioned snapshot projection. Incremental refresh is capability-gated: use documented MCP resource/update notifications when the host advertises them; otherwise refresh the snapshot at a bounded interval while the Dock is visible. The fallback must preserve revision ordering, response limits, and stale-state semantics. It must not invent undocumented host push behavior.

The default snapshot and embedded-App MCP path:

- communicates over the STDIO channel launched by Codex;
- opens no localhost port;
- owns no background daemon beyond the host-managed MCP process;
- exposes no Dock mutation tool in this phase;
- remains functional when HTTP listeners are prohibited by local policy.

### 8.2 Codex right-Browser activation and reopen

The plugin exposes one explicit right-side workflow rather than claiming a permanent native sidebar contribution:

1. The user selects the DevMap plugin/default prompt or asks to open or reopen DevMap on the right.
2. The skill calls `devmap_start_browser_dock` once. The read-only tool starts a loopback Viewer only when none is healthy and otherwise reuses the existing process-owned instance.
3. The tool returns a short accessible summary plus `url`, `revision`, and `reused` in `structuredContent`. The authenticated URL is treated as ephemeral sensitive local data and is not repeated in chat text.
4. In Codex, the skill calls the documented app-level Browser opener with `placement: right` and the returned URL. Other hosts present the URL through their documented local-app surface.
5. Closing the Browser tab does not mutate DevMap state. Repeating the workflow reopens the same healthy Viewer. When the MCP process or Codex task ends, the Viewer stops; after restart, repeating the workflow creates a fresh URL.

The workflow must not use Windows UI automation, DOM injection, private Codex APIs, a manually launched terminal server, or a fixed port. If the host lacks a documented Browser opener, DevMap reports the authenticated local URL as a fallback without claiming that it opened a pane.

### 8.3 Browser fallback

`devmap view --live` extends the already specified temporary localhost Viewer:

```text
GET /api/v1/dock/snapshot
GET /api/v1/dock/events?after=<revision>   # SSE
GET /api/v1/health
```

The service:

- binds only to `127.0.0.1` on a random port;
- requires the Viewer session's ephemeral token;
- embeds all frontend assets and uses no CDN;
- exposes no write endpoint in this phase;
- stops when the CLI process exits;
- sends a complete snapshot on first connection and bounded deltas afterward;
- periodically reconciles Git worktrees and local records so missed filesystem notifications self-heal;
- limits itself to 256 worktrees, 2,048 presence records, and 64 KiB per record, reporting truncation visibly rather than allocating without bound.

SSE is selected instead of WebSocket because the data flow is one-way and read-only. Reconnection uses the last delivered in-process revision; after process restart the client requests a fresh snapshot. This HTTP server starts only when the Browser fallback tool is explicitly called or the user explicitly runs `devmap view --live`.

## 9. Dock interaction

The Dock uses one unified Git relationship map. The source/base target is the left trunk, worktree branches occupy the center, associated chats or Agents attach to their workspace lane, and the selected merge target appears again on the far right. This repeated right-side target makes the return state explicit rather than requiring the user to interpret a badge.

- **Current Worktree** is visually distinct and remains in the first lane.
- **Merged branches** reconnect to the right-side target with a continuous green edge and check label.
- **Unmerged branches** end before the right-side target with an amber arrow and exact ahead/behind counts.
- **Dirty workspaces** receive a red secondary marker without changing merge semantics.
- **Active, idle, and waiting chats** use distinct node colors and visible labels.
- **No linked chat**, **Uninstrumented**, and explicitly **Ignored** are separate gray states; DevMap never treats them as synonyms.

Each workspace lane shows branch, shortened workspace path, short HEAD, merge target, merge state, ahead/behind counts, host, status, confidence, optional route, last activity age, Capture Grade, and gap/blocker counts. `CAPTURE INCOMPLETE` is prominent when the Grade or journal state requires it.

Selecting an Agent focuses the corresponding worktree/route subgraph in the shared Viewer and highlights its latest evidence chain. Navigating to a host chat is an optional `host_navigation` capability. Without it, selection still works inside DevMap and the UI presents the worktree path and route identifier.

The Dock never creates a user-specific persisted layout and never changes the graph merely because an Agent is selected.

At 520 px or wider the map uses the full left-to-right relationship chain. At 320–519 px each workspace becomes a compact vertical lane with the same order: base target, branch/workspace, chat/Agent, merge target. Labels truncate with an accessible full value, edges never overlap text, controls remain visible, and the document has no horizontal page overflow or nested full-height scrollbar.

## 10. Codex and other hosts

Codex integration advertises presentation capabilities separately from capture capabilities:

```text
embedded_mcp_app
integrated_browser
host_navigation
panel_persistence
```

Only observed capabilities may be reported. The presentation selector chooses the first supported surface from the ordered fallback list. A Browser URL is available only after the on-demand Viewer starts successfully; it is not prestarted by the MCP App path.

Claude and Generic MCP reuse the same Presence Schema and Viewer. Their default presentation may be a browser panel or external browser. Host-specific metadata may select an icon or label but cannot change status semantics.

DevMap does not depend on Codex's private internal Agent registry. It shows sessions that produced DevMap events. A host-native task list may enrich the read model only through a documented, capability-gated adapter and must retain its source label.

## 11. Failure behavior

- Invalid or oversized Presence records are ignored, counted, and surfaced as `PRESENCE INCOMPLETE`; they do not crash the Viewer.
- A corrupt Phase 1B journal marks that session `capture_incomplete`; Presence cannot repair or overwrite it.
- A disappeared worktree becomes stale until Git reconciliation confirms removal; evidence is never deleted by the Viewer.
- If the host-managed MCP process fails to start, Codex surfaces the integration failure and DevMap offers the Browser fallback; it must not silently claim that the Dock is live.
- If documented MCP updates are unavailable or interrupted, the MCP App falls back to bounded visible-only snapshot refresh and displays the age of the last accepted revision.
- A disconnected SSE client shows an offline banner and the age of its frozen snapshot.
- If the Codex pane cannot open and the Browser fallback is selected, DevMap starts the temporary Viewer on demand and presents its authenticated localhost URL.
- If Viewer startup fails, `devmap_start_browser_dock` returns a typed tool error and leaves snapshot and embedded-App capture available; it never reports a stale URL.
- If Codex queues rather than immediately displays the Browser tab, the skill reports the queued state accurately and does not start a duplicate Viewer.
- If route data is absent, the Dock still lists the worktree and Agent without inventing a route.
- If no development target can be resolved, merge state is `unknown`; DevMap does not guess from branch names.
- If no exact chat/worktree association exists, the map shows `No linked chat`; it does not attach the most recent task heuristically.
- Absolute paths and local session identifiers are never sent off-machine by this feature.

## 12. Security and privacy

The MCP App path uses the host-managed STDIO channel and opens no network listener. The Browser fallback inherits the existing Viewer security contract: loopback only, random port, ephemeral token, embedded assets, strict content security policy, no arbitrary file reads, and no raw transcript by default.

The Dock adds these rules:

- no mutation API;
- no browser-provided filesystem path parameters;
- no raw Hook payloads in the API;
- no network calls from bundled frontend assets;
- no remote Presence upload in the MVP;
- no automatic task navigation unless the host explicitly declares and authorizes that capability.

## 13. Performance and lifecycle

Neither transport creates a persistent DevMap daemon. Codex owns the lifetime of the STDIO MCP process; a tool-started Browser Viewer is owned by that MCP runtime and an explicitly launched Viewer is owned by its CLI process. Both exit with their owner. The target is an initial Dock snapshot within one second for 100 worktrees and 1,000 bounded Presence records on a normal local SSD, followed by visible updates within two seconds of an accepted local event.

The reducer caches stable repository and worktree identity, reads changed bounded records incrementally, and performs a slower full reconciliation periodically. Expired or old records may be excluded from the read model but are not destructively deleted by the Viewer.

## 14. Verification

Required automated coverage includes:

- two linked worktrees with multiple Codex, Claude, and Generic MCP sessions;
- deterministic current-worktree selection and ordering;
- SessionStart, activity, TurnCompleted, SessionEnd, lease expiry, and recovery transitions;
- no `completed` state without explicit SessionEnd;
- missing adapters shown as `unknown`, not absent;
- corrupt, oversized, replaced, and concurrently updated Presence records;
- no prompt, command, patch, tool input/output, or transcript canaries crossing into Presence or API output;
- plugin-configured Codex launch of `devmap mcp` without a manual command;
- no TCP listener in the default MCP App path;
- `devmap_start_browser_dock` is the only MCP Dock tool allowed to open a loopback listener, reuses one healthy Viewer per MCP runtime, and stops it with the runtime;
- capability-gated MCP updates and bounded visible-only refresh fallback;
- localhost/token/read-only HTTP boundaries and SSE reconnect behavior;
- deterministic development-target selection and exact Git ancestry checks for merged, unmerged, ahead, and behind states;
- exact chat/worktree association, with no path-only or recency-only guessing and separate unlinked, uninstrumented, and ignored states;
- identical graph revision and global layout with or without the Presence overlay;
- Codex MCP App, right-side integrated Browser reopen, queued-open reporting, and external Browser fallback selection from declared capabilities;
- a real disposable repository containing at least two linked worktrees;
- responsive Git relationship-map checks at 320, 360, 520, 736, and 1,024 px, including the repeated right-side merge target;
- performance gates at the limits stated above.

Manual visual acceptance checks verify the right-pane layout at narrow and wide widths, keyboard navigation, screen-reader labels, stale/offline states, `CAPTURE INCOMPLETE`, and graph focus after Agent selection.

## 15. Delivery sequence

The feature is delivered without blocking on the full topology implementation:

1. Presence Schema, bounded atomic store, and reducer.
2. `devmap agents --json` diagnostic projection for host-neutral verification.
3. Revisioned `DockReadModel`, merge-target projection, exact host-session association, and transport-neutral Git relationship-map frontend.
4. `devmap mcp` STDIO bridge, plugin registration, and host-selected MCP App conformance.
5. On-demand `devmap_start_browser_dock` plus `devmap view --live` snapshot/SSE Browser fallback and Codex right-Browser handoff.
6. Route enrichment when Phase 1C records exist.
7. Topology focus integration when the shared graph Viewer is available.

Steps 1–5 provide immediate current/other-worktree visibility without requiring manual server startup in Codex. Steps 6–7 enrich navigation without changing Presence authority.

Cross-machine team Presence, hosted synchronization, PM mutation, always-on-top native windows, and automatic opening of unrelated host chats are explicitly deferred.

## 16. References

- OpenAI, Codex/ChatGPT integrated Browser and localhost application preview: <https://learn.chatgpt.com/docs/browser>
- OpenAI, Codex MCP configuration and local STDIO server command lifecycle: <https://learn.chatgpt.com/docs/extend/mcp>
- OpenAI, plugins containing MCP servers and optional UI: <https://learn.chatgpt.com/docs/build-plugins>
- OpenAI, ChatGPT and Codex changelog entries for conversation side-pane tabs and MCP Apps in the right pane: <https://learn.chatgpt.com/docs/changelog>
