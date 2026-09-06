# DevMap interaction refinement acceptance

Date: 2026-09-06. Base: `origin/main` / `42ba01e6cf301d864278fe7cc3e4d5644951ffc7`.
Branch: `codex/devmap-interaction-refinement`. Changes remain uncommitted in an isolated worktree.

## Delivered behavior

- Workspaces is directly accessible; a branch/path filter and optional list sit above the map. Selection closes the chooser; reopening it collapses the inspector. More expands in normal flow and preserves keyboard focus.
- One bottom inspector provides sticky title/actions, Collapse/Expand, Larger/Smaller and explicit Close. Refresh retains collapsed/size states, selected identity and exact action focus, including shared HEADs and multiple future destinations.
- Recorded future lines and arrivals receive a selected-route accent. Destination links remain beside workspace tasks. Plan details lead with destination, status and milestones and offer an explicit retained target-branch locator.
- Existing history geometry, future arrival geometry, Git facts, task navigation, v3 compatibility and snapshot validation remain unchanged. No parent branch is inferred from a common ancestor.

## Automated evidence

- Baseline: 90 core + 82 renderer tests passed. Initial Rust build exhausted disk; Cargo cleaned only this run's generated build output. Debug info and incremental compilation were disabled and build output relocated to `D:/Tg0 Project 2026/AI-git/devmap-interaction-build`.
- Full Rust suite: 286 passed, zero failed (`cargo test --quiet` with the above target and profile overrides).
- Final focused Rust UI contract: 13 passed after the final responsive adjustment.
- Final Node suites: `node tests/metro_core.cjs` 90 passed; `node tests/dock_renderer.cjs` 87 passed.
- Five added renderer tests cover filtering/refresh, inspector collapse/size, retained target navigation, shared-HEAD chooser/control refresh and multi-destination action identity. New tests were observed failing before implementation/fixes.
- Independent reviewer reproduced and then verified repairs for shared-HEAD chooser closure, shared/arrival header focus loss and second-destination focus substitution. Final verdict: no remaining actionable findings.
- `git diff --check` passed. Design scan reported toolbar inset and inherited cream palette: toolbar inset corrected to 8px; palette intentionally retained from DESIGN.md.

## Browser evidence

Browser: Codex right-side Browser, controlled through CUA. Tested actual renderer at 390×844 and 1280×960, plus default 727×793. Temporary viewport override reset afterward.

- At 390×844, page width/height matched viewport, including enlarged details and expanded More. More's computed position was static; opening it collapsed details without moving focus from More. Escape returned to its trigger.
- Verified branch/path filtering to one result, list closure on selecting that workspace, selected task roster, explicit target-branch navigation to the actual retained `main` commit, Larger/Smaller and Escape collapse.
- Narrow origin hash is omitted visually (full value retained in accessible label/title) so common ancestor, HEAD and the ordinary planned destination fit together. Additional/long endpoints remain horizontally reachable.
- The final compiled Rust viewer also loaded the real repository: 12 observed worktrees, this new worktree at `42ba01e6`, no recorded destination and incomplete task inventory. These are shown as unknown/unrecorded, never fabricated to resemble the fixture. The installed plugin was not changed.

Screenshots are actual browser captures, not generated mockups:

- [Before, synthetic fixture](assets/2026-09-06-interaction/before-fixture.png) — baseline renderer and existing development-overview fixture.
- [Desktop planned arrival](assets/2026-09-06-interaction/desktop-fixture.png) — 1280×960, synthetic acceptance data, selected future route and destination inspector.
- [Narrow task details](assets/2026-09-06-interaction/narrow-fixture.png) — 390×844, synthetic acceptance data.
- [Live repository](assets/2026-09-06-interaction/live-workspace.png) — default 727×793, compiled viewer reading the actual new worktree.

Fixture source: `tests/fixtures/metro/development-overview.json`; observation timestamps refreshed solely in the temporary local preview to exercise fresh inventory states. Multiple destinations and missing targets additionally covered by runtime tests. No test task deep links were opened.

## Resource allowance and scope

Final embedded HTML: 170,371 bytes. Baseline was 163,826 bytes, only 14 below its 160 KiB regression ceiling. The documented internal asset ceiling is now 176 KiB; the existing 512 KiB MCP resource cap and all snapshot validation limits remain unchanged. Rust production diff is a comment describing this allowance; the runtime data producer and layout core were not edited.

No commit, push, merge or plugin reinstall was performed. The local compiled viewer is a review preview, not an installed release.
