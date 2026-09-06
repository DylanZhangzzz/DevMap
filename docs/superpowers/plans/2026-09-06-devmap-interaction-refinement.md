# DevMap interaction refinement

**Goal:** Refine the existing map's header and selected-item inspector while making recorded future merge destinations easier to follow.

**Baseline:** `origin/main` at `42ba01e`. Isolated branch `codex/devmap-interaction-refinement`. User approved writing the plan and proceeding in this session.

**Architecture:** Keep Rust producers, snapshot validation, task identity and `assets/metro-core.js` layout unchanged. Refine the self-contained HTML renderer and existing inspector. History and future arrival zones remain separate: a common ancestor does not establish a parent branch, and a plan does not establish a completed merge.

**Stack:** Rust, dependency-free HTML/CSS/JavaScript, Node renderer contract tests, real Browser inspection.

## Scope and acceptance

- [x] Move Workspaces to the primary toolbar; move its optional list above the canvas with an accessible branch/path filter. Choosing a workspace closes this chooser. More tools use an in-flow disclosure rather than covering the map.
- [x] Keep compact journey identity and endpoint links above the canvas. Give recorded planned destinations a distinct dashed treatment; highlight the selected workspace's future line without changing commit topology. Destination inspection offers navigation to a retained target ref; missing targets remain unavailable and no parent relationship is inferred.
- [x] Unify inspector headers for workspace, shared HEAD, commit, task and route. Provide Collapse/Expand (selection retained), Larger/Smaller (bounded height), and Close (explicit selection dismissal). One content scroll area with sticky controls. Refresh preserves mode, scroll and keyboard focus. Explicit new selection opens collapsed details.
- [x] Keep existing exact task titles/navigation, observation uncertainty, v3 compatibility, graph geometry and safe DOM text construction. Source verification found the actual baseline HTML budget was 160 KiB (14 bytes free), not the stale 144 KiB DESIGN note. Increase only this asset regression budget to 176 KiB to fit the new UI; retain the existing 512 KiB runtime resource cap and all snapshot validation limits. `src/dock_asset.rs` changes only its explanatory comment; `tests/dock_ui_contract.rs` documents the bounded asset allowance.

## Implementation and verification

1. Add failing runtime contracts to `tests/dock_renderer.cjs`: chooser filtering and refresh, selection closes chooser, collapse/size survives refresh, Escape collapse without losing workspace/viewport, selected plan paths and real target navigation. Keep existing semantic and security tests.
2. Modify only `assets/dock.html` for these behaviors. Reuse existing `renderOverview`, `renderJourney`, `showDetails`, `selectLane` and `showRoutePlan`; avoid a new framework or transport.
3. Run both Node suites and Rust tests. Baseline Node: 90 core + 82 renderer passing. Initial Rust baseline interrupted by disk exhaustion; use a bounded build with debug information/incremental output disabled and report its outcome.
4. Serve the actual embedded UI in the right-side Browser and inspect an explicit test fixture with multiple destinations alongside current repository data. Inspect narrow and desktop layouts, keyboard dismissal, enlarged/collapsed inspector, long names, unknown observations and unchanged future/history geometry. Save actual screenshots and document the validation in `docs/audits/`.
5. Update the affected selection/navigation rules in `DESIGN.md` and review the final diff. No commit, push, merge or plugin installation is required by this request.
