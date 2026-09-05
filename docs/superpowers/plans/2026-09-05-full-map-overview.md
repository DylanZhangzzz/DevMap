# Full-map overview

Work package B of the map review. The user requested committing/pushing work package A first, then designing and implementing the overview. A was pushed as cafea0e on codex/devmap-journey-focus. This increment remains on that branch for review.

## Problem and design

The previous Full map scaled the detailed card layout down uniformly. In the audit fixture, three commits and two worktrees yielded about 23% scale; actual lines and arrival labels became tiny while fixed-size workspace markers and the default workspace list dominated the screen.

Use a separate display projection of the existing actual graph. Give current HEADs, forks, merges and history boundaries readable spacing. Draw ordinary commits as small points; retain key stations as accessible controls. Place compact worktree markers below the rail band, group shared HEADs, and expose exact worktree choices on activation. Show the workspace inventory only when requested.

Planned arrivals remain readable controls outside the actual graph, connected by dashed future intent. Their source is the observed worktree HEAD, or the worktree marker when no HEAD can be placed. They never represent an actual merge commit. The detailed view and its existing common-ancestor/unknown-origin semantics remain unchanged.

## Implemented behavior

- Pure Core.projectTopology preserves every node, edge identity, parent endpoint and boundary fact; all original rail bends use the same monotonic coordinate mapping. Crossing gaps are recomputed. No history folding, new dependency, Git operation, MCP schema or tool is introduced.
- Overview uses fixed-size text and rail strokes instead of scaled detailed cards. Its zoom label says Overview; zooming in returns to readable detail.
- Compact markers retain the name, visible HEAD hash, passenger count or unknown state, and necessary risk. Full paths and branch identities remain accessible. Unplaced worktrees remain present without fabricated stations.
- Fork/merge/boundary and worktree HEAD controls open the detailed commit and transfer keyboard focus. Arrival controls reuse exact-source navigation. Refresh retains marker/arrival focus and the optional inventory's expansion and scroll state.
- The inventory starts closed. A desktop scrollbar gutter avoids unnecessary horizontal scrolling. Narrow panes keep a minimum readable diagram width and allow horizontal pan; dense worktree labels can require vertical scrolling.

## Verification and stopping point

- 149 Node core/renderer tests passed, including RED/GREEN regressions for projection identity, clustered HEAD spacing, optional inventory, arrival navigation, unborn-plan association and keyboard focus transfer.
- 13 Rust UI/resource tests and 3 plugin tests passed. cargo fmt --check and git diff --check passed.
- Read-only review found two actionable regressions (unborn plan association and station focus); both were fixed and re-reviewed.
- Browser verification at 1280 x 720 and a 480 x 800 example pane. The actual repository retained 87 nodes and 87 edges in both detailed and overview DOM, with all 11 worktrees represented by 10 markers. Desktop scroll width matched available width; marker text stayed at 12 px.
- The three-commit/two-worktree synthetic fixture shows its current platform, comparison platform and explicit main destination together on desktop. Narrow pan exposes the complete destination label; keyboard activation returns to the exact source worktree and its persistent journey summary. Synthetic task links were not opened.
- Screenshots in target/verification: overview-real.png, overview-demo.png, overview-narrow.png, overview-narrow-arrival.png. The actual repository preview is Git-only and labels passenger observations unknown.

The embedded UI regression budget is 160 KiB (previously 148 KiB); the 512 KiB transport cap is unchanged. The remaining need to pan in narrow panes or scroll a dense set of platforms is explicit. The observed improvement does not justify adding history folding or replacing the graph renderer now. This work does not claim reliable creation-event capture, passenger-history replay or automatic Git execution.
