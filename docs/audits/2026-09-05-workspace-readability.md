# Workspace topology readability

Implemented the approved local UI refinement in `assets/dock.html`.

- Open at the current workspace at 100% instead of fitting the entire commit graph automatically. Preserve user zoom and position on refresh; use Full map explicitly for the whole graph.
- Replace numbered overview circles and unexplained exclamation marks with fixed-size, named workspace labels, branch and observed task counts, explicit status text, and a visible action.
- Expand shared-HEAD locations into separate named workspace choices. Selecting one locates that exact checkout. Refresh preserves the chooser and updates its facts.
- Keep the commit-history, workspace/reference-link, and commit legend visible.
- Separate intervention flags from ordinary uncommitted changes in the summary copy.
- Keep labels inside the overview canvas and separate overlapping labels vertically; scroll to additional locations. Current-workspace names and branches wrap instead of clipping.
- Strip template source indentation when embedding the resource to retain the existing 128 KiB limit; preserve JavaScript newlines and the unchanged topology core.

## Verification

- `node --test tests/dock_renderer.cjs tests/metro_core.cjs`: 120 passed.
- `cargo test --test dock_ui_contract --test dock_viewer`: 19 passed.
- Headless Edge: 360, 560 and 1280px widths, initial view, full map, shared-workspace chooser and exact-checkout activation. No runtime errors, page horizontal overflow or overlapping overview labels. Evidence: `assets/2026-09-05-readability/`.
- Screenshots use a synthetic six-workspace fixture, not live user task state. The final radius-only alignment changes 8px to the established 6px token.
- Impeccable detector: the new radius was aligned to DESIGN.md; the warm background was retained from the established product palette.

This changes the local development source and tested build. It does not replace the installed plugin executable or restart the currently open Viewer.
