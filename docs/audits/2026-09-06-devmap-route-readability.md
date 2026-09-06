# Route readability refinement

User approved replacing compressed multi-turn paths with readable direct tracks. Continued in the existing isolated `codex/devmap-interaction-refinement` worktree, preserving the earlier inline-summary and inspector changes.

## Change

- The v4 renderer retains topology identities/lane assignment but no longer projects the old workspace-layout detour geometry into a narrow strip.
- At 100% scale, parallel historical tracks have 24 CSS pixels between centers. Ordinary continuous chains are straight; ordinary branch transitions use a single bend with a diagonal entry or exit. A shortcut that would hit another station uses an extra channel.
- A stable topological ordering separates commit rows. History-only adjacent rows use 48px; rows adjoining workspace HEADs reserve 96px. Summary expansion remains independent of commit geometry.
- Canvas bounds include tracks even when there are no workspace attachments. Horizontal overflow remains available when the graph and readable labels exceed the viewport.
- Crossing gaps use actual segment intersections, including diagonal connections. Offscreen endpoint navigation uses segment/rectangle clipping and distinguishes a visible diagonal from a nearby diagonal that misses the viewport.
- Locate positions include the station ring and workspace label. Labels remain below the historical tracks in the horizontal layout.

## Verification

- Both Node suites: 185 tests passed, covering minimum track spacing, simple turns, real endpoints, diagonal intersections, no unrelated station pass-through, merge shortcuts, deterministic input ordering, unanchored history bounds, viewport clipping and existing summary/refresh behavior.
- Rust embedded UI contract suite: 13 tests passed on final source, including the unchanged 176 KiB embedded asset allowance. `git diff --check` passed.
- Read-only review found unanchored canvas clipping and outdated orthogonal viewport intersection logic. Both received failing regression cases and fixes; the reviewer independently confirmed both fixes with no remaining findings in that review scope.
- Actual Browser checks: 390×844 narrow history and 1440×1000 horizontal overview/summary using the existing synthetic development fixture. Track separation, branch turns and visible crossing gaps were inspected directly. Fixture timestamps were refreshed only by the temporary preview server.

## Captures

- [Narrow history](assets/2026-09-06-routing/narrow-history.png)
- [Wide history](assets/2026-09-06-routing/wide-history.png)
- [Wide summary](assets/2026-09-06-routing/wide-summary.png)
- [Compiled live repository preview](assets/2026-09-06-routing/live-workspace.png) — final executable, default right-side Browser, exact refinement workspace selected.

These are browser captures of test data. They do not add route plans to the live repository. Native scrolling is intentional; a narrow viewport is not forced to show all branches and full-width labels at once. No commit, push, merge or plugin reinstall is part of this delivery.
