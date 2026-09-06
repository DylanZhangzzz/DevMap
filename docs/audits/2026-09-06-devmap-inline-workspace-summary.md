# Inline workspace summary acceptance

Implemented in the existing `codex/devmap-interaction-refinement` worktree, continuing the first interaction refinement. This is a local review preview; no commit, push, merge or plugin reinstall was performed.

## Behavior

- A workspace opens one inline summary. Shared HEADs first offer exact checkout choices. Selecting another replaces it; repeat click and Escape collapse it.
- The selected label retains working state; its summary provides path, an early full-detail action, recorded destinations and two current tasks with exact task navigation. Long content scrolls inside a bounded reservation.
- Full workspace details use the existing bottom inspector. Historical commit coordinates and rails stay unchanged; labels and association stems reflow around reserved summary space.
- Same-summary refresh preserves scroll and focused action. A new workspace starts at the top. A task removed from the preview returns focus to the summary. Unavailable and partial plan inventories remain explicit.

## Verification

- `node --test tests/dock_renderer.cjs tests/metro_core.cjs`: 181 passed.
- `cargo test --quiet --test dock_ui_contract`: 13 passed, including the existing 176 KiB embedded resource allowance. No further budget increase.
- `git diff --check`: passed; Git emitted only line-ending conversion notices.
- Read-only reviewer identified three issues (destination uncertainty, scroll transfer, disappearing task focus); all fixed and independently rechecked with no remaining findings in that review scope.
- Actual Browser acceptance at 390×844 and 1120×860: shared-HEAD choice, readable wrapped task names and action cue, inline summary, Escape and full-detail handoff. Compiled viewer also verified against the real repository in the default right-side Browser.

## Captures

These are actual browser screenshots. Fixture images use `tests/fixtures/metro/development-overview.json` with timestamps refreshed in the temporary preview only. They demonstrate future intent, task states and shared HEADs; they are not claims about live repository destinations. The full-detail capture also shows stale observations remaining qualified after time elapsed.

- [Narrow inline summary](assets/2026-09-06-inline/narrow-summary.png)
- [Wide inline summary](assets/2026-09-06-inline/wide-summary.png)
- [Explicit full details](assets/2026-09-06-inline/full-details.png)
- [Live repository summary](assets/2026-09-06-inline/live-workspace.png)

The current live repository preview reports no recorded destination and an unconfirmed task inventory. No parent branch, future route or task record was fabricated for that preview.
