# DevMap Rail View Design QA

## Evidence

- Source visual truth: `.superpowers/brainstorm/product-design/02-rail-view-source.png`
- Existing product screenshot: `.superpowers/brainstorm/product-design/01-current-ui.png`
- Implementation screenshot, iteration 1: `.superpowers/brainstorm/product-design/viewport-1024.png`
- Combined comparison, iteration 1: `.superpowers/brainstorm/product-design/comparison-source-vs-implementation-v1.png`
- Implementation screenshot, iteration 2: `.superpowers/brainstorm/product-design/implementation-v2-1024.png`
- Narrow implementation screenshot, iteration 2: `.superpowers/brainstorm/product-design/implementation-v2-375.png`
- Combined comparison, iteration 2: `.superpowers/brainstorm/product-design/comparison-source-vs-implementation-v2.png`
- Source screenshot: 748 × 794 px at device scale 1.25, cropped by 34 px to remove the brainstorming host header and normalized to 1,024 px width for comparison.
- Implementation screenshot: 1,024 × 900 px at a 1,024 × 900 CSS viewport, device scale controlled by the in-app Browser.
- State: real local repository, dark theme, MAP density, four worktrees, no supplied Codex task inventory.

## Full-view comparison, iteration 1

The implementation captures the approved hierarchy: one fixed `main` rail, compact branch/worktree rows, MAP/READ/FULL controls, small-area semantic colors, and a neutral dark canvas. It is materially less card-heavy and less saturated than the existing product screenshot.

### Findings

- **P1 — Branch rails do not visibly connect to the integration rail.**
  - Location: `.branch-rails`, `.fork-group`, `.fork-node`.
  - Evidence: the source uses vertical fork paths from `main` into parallel branch rails; iteration 1 shows independent branch rows with hash pills but no ancestry connector.
  - Impact: users can read row state but must infer that each row actually forked from the target rail.
  - Fix: add one restrained vertical ancestry spine per target and a short connector from each fork node into that spine; preserve explicit hash labels.

- **P2 — Summary metrics retain the old card hierarchy.**
  - Location: `.overview`, `.metric`.
  - Evidence: iteration 1 uses three large surface cards above the graph; the source keeps branch count and state secondary to the topology canvas.
  - Impact: the dashboard summary competes with the graph for first attention, especially at 375 px where it occupies most of the opening viewport.
  - Fix: render metrics as a compact inline summary strip with separators and no raised surface.

- **P2 — Selection detail is visually detached from the map.**
  - Location: `.map-frame`, `#selection-details`.
  - Evidence: the source attaches the selected-worktree drawer to the bottom of the map shell; iteration 1 renders it as a separate card below.
  - Impact: selection feels like a separate panel instead of a detail level of the same graph.
  - Fix: move the existing semantic detail section inside `.map-frame` and style it as a border-top drawer.

## Focused comparison

Focused review used the header/summary region, one fork/branch row, and the selection drawer because those are the fidelity-critical regions. Typography is close in weight and hierarchy; the implementation intentionally keeps the existing product's dark theme while adopting the source's neutral surfaces and blue topology accent. No imagery or non-standard icons are present in either artifact. App-specific copy is preserved except for the approved `Repository topology` / density labels.

## Required fidelity surfaces

- **Fonts and typography:** system UI and monospace metadata match the compact developer-tool intent; long branch names remain readable. No P1/P2 typography issue.
- **Spacing and layout rhythm:** branch rows are compact, but the three findings above require one layout refinement pass.
- **Colors and visual tokens:** graphite surfaces and restrained blue/green/amber/red usage satisfy the approved color direction. Contrast still needs automated/browser verification after the refinement.
- **Image quality and assets:** no raster imagery, logos, illustrations, or custom icon assets are required by this data-centric screen.
- **Copy and content:** branch, worktree, merge, dirty, ahead/behind, and unknown text remain explicit. Task content is absent because the standalone Viewer was not supplied a Codex inventory.

## Interaction evidence

- MAP, READ, and FULL each update the page density and pressed state.
- Selecting a worktree updates `#selection-details`.
- Browser fallback refresh reports that Git refreshed while task names were not resynced.
- 375, 594, 736, and 1,024 px checks report no document-level horizontal overflow.
- Browser console warnings/errors: none.
- Branch-disclosure interaction is not visible with the current four-worktree repository and remains covered by the bounded artifact contract plus a later fixture/manual run.

## Iteration history

### Iteration 1

- Earlier findings: missing ancestry connector (P1), card-heavy summary (P2), detached selection drawer (P2).
- Fixes made: added a continuous ancestry spine and fork connectors, converted metric cards to a compact summary strip, and attached the detail inspector to the map shell.
- Post-fix evidence: `.superpowers/brainstorm/product-design/implementation-v2-1024.png` and `.superpowers/brainstorm/product-design/comparison-source-vs-implementation-v2.png`.

### Iteration 2

- Earlier findings checked: ancestry connector (P1), card-heavy summary (P2), detached selection drawer (P2).
- Result: all three are resolved in the same 1,024 px MAP state. The source places synthetic branches at different horizontal commit positions; the real implementation uses exact fork hashes plus a shared ancestry spine because the current read model does not expose a complete commit-timeline scale. This is an intentional truthful-data constraint, not an unresolved visual mismatch.
- Narrow evidence: at 375 px the summary strip is 21 px high, the graph reflows to a vertical tree, all four branch rows remain present, and document overflow is false.
- Contrast evidence: text/canvas 16.84:1, muted/surface 6.84:1, accent/canvas 7.53:1, success/canvas 8.88:1, warning/canvas 9.35:1, danger/canvas 6.44:1.
- Console warnings/errors: none.
- Remaining P3: the standalone Viewer cannot demonstrate task-card density or the seven-plus-branch disclosure because no Codex task inventory is supplied and the repository has four worktrees. The controls and bounded rendering path remain covered by artifact contracts; final installed-plugin QA can exercise richer live data.

## Final result

final result: passed
