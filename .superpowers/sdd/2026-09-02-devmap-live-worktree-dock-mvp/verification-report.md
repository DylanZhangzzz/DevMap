# DevMap Live Worktree Dock MVP verification

Date: 2026-09-02  
Branch: `codex/devmap-live-worktree-dock`

## Result

The MVP acceptance suite passes for local multi-worktree discovery, Presence projection, a shared bounded read model, Codex MCP App packaging, the host-managed STDIO transport, the explicit Browser fallback, privacy boundaries, source-Git immutability, and release latency targets.

The Codex-side integration was verified by launching the exact plugin-configured `devmap mcp` command in a real fixture repository, negotiating MCP, listing the Dock tools, reading the bundled UI resource, and confirming the runtime audit opened zero TCP listeners. The already-running development task cannot hot-load a newly created repository plugin, and this verification did not create a separate user-owned Codex task or modify global plugin installation state. Therefore the side-pane host chrome itself was not re-screenshoted; the same bundled HTML resource was rendered and exercised through the authenticated Browser fallback.

## Executable acceptance evidence

- `tests/live_dock_acceptance.rs` creates one repository and two linked worktrees. Real Codex and Claude hook events plus a real Generic MCP Decision call produce one shared model with `working`, `completed`, `stale`, and uninstrumented `unknown` entries.
- Only Claude `SessionEnd` produces `completed`; an expired Generic MCP lease produces `stale`; `NoRoutes` leaves every `route_id` empty.
- A corrupt Presence file and corrupt journal tail produce `presence_record_invalid`, `journal_corrupt`, and `capture_incomplete` without crashing.
- Prompt, command, patch, tool-input, tool-output, and transcript canaries do not appear in Presence, `agents --json`, MCP tool output, the MCP HTML resource, the HTTP snapshot, or SSE output.
- Source HEAD, branch, index, refs, config, stash, remotes, and worktree registration are unchanged by capture and all read paths.
- The configured plugin command launches over STDIO with no manual HTTP process. `McpRuntime::audit()` reports zero TCP listeners.
- Browser endpoints accept only authenticated `GET` requests, use `Cache-Control: no-store`, reject unknown/traversal-like paths, and close their listener during runtime shutdown.

## Performance evidence

`cargo test --release --test live_dock_acceptance -- --nocapture` measured:

- 100 worktree descriptors and 1,000 Presence records reduced in 9.73 ms (target: under 1 second).
- A new linked worktree became SSE-visible in 658.80 ms (target: under 2 seconds).

## Visual and interaction acceptance

The bundled asset was served by `devmap view --live --source .` and inspected in the Codex in-app Browser.

| Check | Evidence | Result |
| --- | --- | --- |
| 320 px pane | Effective content width 305 px equaled viewport width; no horizontal overflow | Pass |
| 520 px pane | Body width and viewport width both 520 px | Pass |
| Touch/keyboard target | Minimum rendered `button`/`summary` height was 48 px | Pass |
| Focus visibility | Focused control had a solid 2.4 px focus outline | Pass |
| Screen-reader semantics | One `main`; Current and Active regions; native expandable stale group; all buttons had labels | Pass |
| Current emphasis | Current row had the accent border and appeared first | Pass |
| Stale default | Native `<details>` group was closed by default and could be expanded | Pass |
| Stable row selection | Selecting the Current row set `aria-pressed="true"`; equal revisions did not replace focused DOM | Pass |
| Live/offline state | Live snapshot showed `LIVE · updated now`; 11 seconds after owner exit it showed `OFFLINE · last update 11s ago` while retaining revision 1 | Pass |
| Integrity state | `CAPTURE INCOMPLETE` was visible on uninstrumented rows | Pass |
| Reduced motion | `prefers-reduced-motion` rules disable meaningful transition and animation durations | Pass |

## Design traceability

| Design section | Implementation or evidence |
| --- | --- |
| 1. Goal | `tests/live_dock_acceptance.rs` |
| 2. Product placement | MCP is the default plugin path; Browser server is explicit only |
| 3. Considered approaches | Decision captured in the design; selected approach implemented by `src/mcp.rs` and `src/viewer.rs` |
| 4. Architecture | `src/worktrees.rs`, `src/presence.rs`, `src/dock.rs`, `src/mcp.rs`, `src/viewer.rs` |
| 5. Storage and authority | Common Git-dir Presence plus per-worktree journals; no canonical graph writes |
| 6. Presence schema | `tests/presence_store.rs` and strict deserialization tests |
| 7. Status semantics | `tests/presence_store.rs`, `tests/live_dock_acceptance.rs` |
| 8. Presentation transports | `tests/dock_mcp.rs`, `tests/dock_plugin.rs`, `tests/dock_viewer.rs` |
| 9. Dock interaction | `tests/dock_ui_contract.rs` and Browser acceptance above |
| 10. Hosts | Codex, Claude, and Generic MCP in `tests/live_dock_acceptance.rs` |
| 11. Failure behavior | Corruption and offline assertions in acceptance and Viewer tests |
| 12. Security and privacy | Canary suite, token/method/path tests, bounded model tests |
| 13. Performance and lifecycle | Release acceptance timings and stoppable Viewer test |
| 14. Verification | Final command gate plus this report |
| 15. Delivery sequence | Eight incremental feature/test commits on the isolated branch |

## Explicit post-MVP boundary

The Dock is local and ephemeral. Cross-machine aggregation, canonical Route reconstruction, PR/Release evidence topology, shared PM graph state, Context Repository ingestion, merge gates, and attestations remain later phases. The MVP never fabricates those facts.
