# Workspace chat cards — second iteration

final result: blocked

The latest code and installed artifact are ready for a confirmation browser pass. Visual acceptance remains pending; the live iteration-two pass below found and corrected interaction defects.

## Why the first iteration stopped too early

- Renderer and geometry tests covered data ownership and interaction wiring, not CSS layout or fidelity to the approved image.
- The old bottom inspector remained in the page layout; task navigation called `showTask()` before opening a task.
- A static file snapshot with zero linked tasks was presented instead of a newly installed, live MCP Viewer. Its refresh controls still attempted network refreshes.
- The running MCP process embedded the previous HTML/JS. A successful Release build did not change that process.
- Browser inspection of the file preview was unavailable, but this missing acceptance gate was not reflected in the completion statement.

## Implemented in iteration two

- One workspace card per worktree, even at a shared commit. Chat titles and direct links appear before technical details.
- Larger blue chat titles, standard Bootstrap chat/arrow/disclosure icons, restrained current-chat badge, and observed-state indicators.
- Two chats visible initially; expand the remaining list inside the card, with bounded height and scrolling for large inventories.
- Workspace summary exposes path, branch, HEAD, working tree and integration inline.
- Selection details float outside normal layout flow instead of reducing the map viewport.
- Direct task navigation dismisses the inspector. Records without a supported navigation path remain inspectable.
- Source resource budget increased from 188 to 196 KiB to include the card presentation and licensed inline icons; no external runtime asset requests.

## Verified evidence

- `node --test tests/dock_renderer.cjs tests/metro_core.cjs`: 198 passed.
- `cargo test --test dock_ui_contract --test dock_plugin --locked`: 16 passed.
- `cargo build --release --locked`: passed.
- `git diff --check`: passed.
- Installed local plugin version: `0.1.1+codex.20260908131058`.
- Independent code review identified a presence-only navigation fallback defect; fixed with a regression test.
- Opening the map in the current task after install still served the old grouped-workspace renderer. The current process is not the installed artifact.

## Blocking browser acceptance

Reference: user-approved `codex-clipboard-ca87c0a2-e42a-4f3e-9481-56a5c9d3a8a3.png`.

Load the installed plugin in a fresh Codex task, or after a user-controlled app restart. Obtain real local unarchived task inventory and an actual current-directory report using the DevMap Skill. Open the MCP-owned live Viewer in the right Browser. Do not substitute a static snapshot or manual web server.

1. Verify runtime Build and chat-card DOM against the installed binary resource; nonzero linked task count and real titles must be present.
2. Capture the reference and real Viewer together at comparable viewport sizes/states. Check 420px, 826px, and a wider view.
3. Typography: title prominence, line height, Chinese glyph fallback, long-title wrapping and current-chat badge positioning.
4. Spacing/layout: card padding, stem ownership, separate shared-HEAD cards, no overlap, usable map viewport and scroll position retention.
5. Colors/tokens: pale surfaces, blue navigation, readable secondary text and truthful state colors.
6. Assets: Bootstrap icons match the intended speech bubble/arrow/chevron roles. No raster imagery is required by this design.
7. Copy/content: live data labels must distinguish current observation, stale observation, incomplete inventory, map source and reported Agent location. No mock/example tasks in live data.
8. Interactions: a chat opens in one action; workspace details expand inline; commit details do not alter viewport height; close/Escape/focus restoration work; more chats remain reachable.
9. Test long titles, at least three chats in a card, empty workspaces, and increased text scale. DOM-only tests are insufficient for these checks.
10. Fix P0/P1/P2 findings, rebuild/reinstall as necessary, capture again and update this report to `passed` only after actual comparison.

## Live acceptance attempt — 2026-09-08 15:24 UTC

Authorization is present. Acceptance task `01a0819c-834c-72f1-bc5d-dedd1ecb9b64` was created and performed this inspection. No commit or push is authorized or performed.

Result remains **blocked**, with a reproduced runtime-selection failure. Creating a new task did NOT pick up the installed MCP configuration in this app session. A host reload/restart is now required before judging iteration-two visuals.

### Verified source, install, and running process

- Development commands ran at `C:/Users/user/.codex/worktrees/b714/AI auto-git context`; `git rev-parse --show-toplevel` agreed. Branch: `codex/visible-workspace-chats`; the six existing modified files were retained.
- Release binary, plugin source runtime, and installed-cache runtime all have SHA256 `FFD913E3B667F51679C95C5ADC0C50B984635573138BF17F1D99B74CFB730645`.
- `codex plugin list` reports `devmap@personal` installed/enabled at `0.1.1+codex.20260908131058`. Both source and cache `.mcp.json` point to that version's executable.
- However, the new Viewer listener on port 56955 is owned by PID 5200, created at 15:21:53 UTC for this task, executing `C:/Users/user/plugins/devmap/.runtime/0.1.1+codex.20260907214114/bin/devmap.exe`.
- Visible page Build is `v0.1.1-3-gdb696768`. The DOM still groups four shared-HEAD workspaces into one `Detached HEAD · db696768 +3` button, rather than rendering separate chat cards. This is evidence of old runtime resources, not a failure of the unreviewed new source.
- The first `devmap_open_map(surface:browser)` exceeded the 10-second tool timeout but did start this MCP-owned Viewer; one retry returned its healthy URL. No manual server or static snapshot was used.

### Browser and task evidence

- Read exact map workspace paths, canonicalized existing paths on Windows, and matched the host's `list_threads(limit:50)` inventory by full path. Coverage is incomplete because the host reached its limit.
- The host list omitted this newly created task; its exact ID/title/registered cwd/active status were verified with `read_thread`. That supported observation supplemented the partial list without inventing a task.
- 18 real linked tasks were displayed across 16 workspaces. Only this task received a working-directory report, checked at Unix time `1788880979`; registered cwd remains `16e0`, reported development location is `b714`.
- `open_in_codex(placement:right)` returned queued. The in-app Browser was then opened and actually inspected through browser tools. Screenshot captured at the native 662 x 792 viewport: `docs/audits/assets/2026-09-08-live-v2/runtime-old.png`.
- Compared against the user-approved reference image. P1 runtime mismatch prevents visual acceptance: default chat titles are absent and shared-HEAD ownership remains grouped in the running old UI.
- The visible details distinguish Map source `16e0` from Agent-reported work `b714`; observation aged to stale while Git remained live, and inventory is marked incomplete.
- `git diff --check` passed. No implementation changed in this acceptance attempt, so earlier test/build evidence remains historical rather than being claimed as a fresh run.

### Resume gate

Reload/restart the Codex host so it launches the installed `20260908131058` executable, then continue this task. Verify the executable path and separate-card DOM before the 420/826/wide viewport and interaction matrix above. Do not merely create another task under the same stale host configuration. Do not overwrite the old versioned binary or terminate unrelated tasks to force a version change.

Pending: all iteration-two visual and interaction acceptance (including navigation, multi-chat expansion, long titles, text scaling, floating inspector, Escape and focus restoration). The old runtime screenshot is diagnostic evidence only, not a delivery screenshot of iteration two.

## Resumed live acceptance — 2026-09-08 15:28–15:35 UTC

The user restarted Codex and requested continuation. The host now launched `20260908131058`, resolving the previous runtime-selection blocker. `devmap_open_map(surface:browser)` created a new MCP-owned Viewer on port 52171. The matching right-side in-app Browser was visibly inspected, not merely queued.

### Real-browser findings

- Separate shared-HEAD workspace cards and real chat titles are now present; the b714 card contains this exact task and `打开最新版 DevMap`. The current-chat badge is visible. Empty workspaces say the list is unconfirmed. No synthetic tasks were supplied.
- The host list was again capped at 50, with this task now included. Only this task received the b714 directory report, checked at Unix time `1788881270`, while retaining its registered 16e0 cwd. Old reports were not re-stamped.
- Captured default 512px rendering, then 826 x 1000 and 420 x 900 layouts. Saved `before-826.png`, `before-420.png`, and `cards-420.png` under `docs/audits/assets/2026-09-08-live-v2/`. These are pre-fix diagnostic captures; `cards-420.png` includes the Escape test state, not an accepted final layout.
- Real multi-chat card: two titles initially, “Show 13 more chats” expanded its real inventory inside the card with bounded scrolling. The button changed to “Show fewer chats”. Long Chinese titles wrapped, and empty cards remained separate.
- P1: initial view followed Map source 16e0 rather than the verified b714 Agent report. Corrected to prefer the fresh exact report and otherwise retain Map source fallback.
- P1: “Locate current Agent” opened a workspace inspector over the desired chat card. Corrected to locate and select the card without opening an inspector.
- P1: resizing between horizontal and vertical layouts lost the selected workspace position. Resize/font reflow now requests selection-anchor preservation.
- P2: Escape collapsed the inspector instead of dismissing it. Escape and Close now dismiss, return focus to the selected map object when present, and clear retained selection so a removed task does not reopen an error panel.
- Commit details were visibly floating. Opening them changed the measured viewport from 610px to 642.8px because an edge-wayfinding row disappeared; it did not shrink the map. The attempted Escape action left the panel visible and height at 642.8px. A clean open/close measurement remains part of the confirmation pass.
- One-click navigation to this exact task was attempted. Browser Use security rejected the `codex://threads/…` navigation. No alternate browser, indirect navigation, or other workaround was attempted. This gate requires a human click; DOM validity and unit tests do not establish actual host navigation success.

### Corrections and validation

- Changes: `assets/dock.html`, `tests/dock_renderer.cjs`; existing iteration-two changes in other files preserved.
- JS renderer/core: **199 passed** after the final changes. Log: `js-tests.txt`.
- Rust plugin/UI checks: **16 passed**. Log: `rust-tests.txt`.
- `cargo build --release --locked`: passed.
- Plugin validation and `codex plugin add devmap@personal`: passed. Log: `install.txt`.
- Installed version: **0.1.1+codex.20260908153418**.
- Release and installed-cache SHA256: **A1CFBB1C46BD9026B18DB67DA9F70485B0BB20C6C54FD779475B52531731D419**.
- Mechanical detector: saved `detector.json`. Static canvas-padding warning does not account for positioned map coordinates; accent stripe follows the approved reference. Remaining token/radius/shadow advisories refer to pre-existing styling and are not evidence of acceptance. No mechanical result substitutes for the confirmation screenshots.

### Remaining confirmation gates

Reload/restart the host to activate `20260908153418`, then verify the four fixes on the new MCP-owned Viewer. Repeat 420/826/wide comparison, workspace detail expansion, text zoom, native scroll and focus retention. Have the user click a real chat link to verify host navigation, since automated navigation is blocked by Browser Use security policy. No final `passed` result is justified yet. No commit/push performed.

## Continued acceptance — 2026-09-08 17:17–17:28 UTC

Result remains **blocked / not accepted**. Development remained exclusively in b714. No commit or push was performed.

### Running version and browser evidence

- Processes 48200 and 41328 execute the installed `20260908153418` runtime. The live MCP-owned Viewer is on port 58120 and was visibly inspected in the task's right-side Browser.
- Initial Agent-card positioning and Locate-without-inspector were observed working. A fresh exact b714 directory check at Unix time `1788888133` renewed only this task's Agent report; registered cwd remains 16e0. The host inventory remains capped at 50 and explicitly incomplete.
- Escape now closes commit details and restores focus to `commit:db696768baf366b442adf6097e0c1f4cec3f7e08`. The map viewport measured 515.6px high before and after dismissal. Evidence: `escape-confirmed.png`.
- Earlier 420px measurement showed a current-chat title squeezed to approximately 46px width by its badge. The heading used flex with a growing title and a fixed-width badge. This is a readability defect.
- Responsive testing has an additional host interference: immediately after a 420 x 900 override, DOM width was 420 and map width 383; the next observation reported window width 627 without another test resize. One 420-to-826 run kept the selected card visible at x=334.4, y=334, while earlier runs lost it. Consequently these screenshots do not establish deterministic breakpoint acceptance. `narrow-title-diagnostic.png` is diagnostic only and must not be labeled a verified 420px final capture. Temporary viewport overrides were reset.

### Corrections and fresh validation

- Current-chat headings now use two grid columns, with the badge on its own row under the title. The badge no longer consumes the title's horizontal space.
- When orientation changes, the map reveals the selected object using the new geometry instead of retaining a potentially clamped old screen offset. Same-orientation reflows keep the existing anchor behavior and map scale is preserved.
- A regression with 20 separate shared-HEAD workspaces reproduces loss of the selected card after simulated browser scroll clamping. It failed on the previous implementation and passes with the correction across 1000, 360, and 826px viewport widths. This tests positioning logic, not browser visual acceptance.
- JS renderer/core: **200 passed**, 0 failed (`js-tests.txt`). Rust UI/plugin contracts: **16 passed**, 0 failed (`rust-tests.txt`). `git diff --check` and `cargo build --release --locked` passed.
- Plugin validation and installation passed (`install.txt`). Newly installed version: **0.1.1+codex.20260908172746**. Release and installed-cache SHA256 both equal **B9DA8D996A53162C3540694092C4614C150A4CD89299F59FCF07519AF55BF3DE**.

### Remaining gates

The current host still runs 153418; 172746 is installed but its new grid and breakpoint handling have not yet been visually verified. Restart the host to activate it, then verify the executable path and capture stable 420/826/wide layouts with width recorded alongside each screenshot. Complete increased-text-scale, inline workspace details, and native scroll/focus checks. A real chat-link host transition still requires a human click because Browser Use rejected automated codex-protocol navigation; no workaround was attempted. Do not declare overall acceptance until these gates have evidence.

## Working-directory persistence correction — 2026-09-08 20:25 UTC

The user identified task `01a081a1-751a-7473-aa3b-91995c128f7e` at the original directory's historical HEAD. Its host registration remains the project root at `50bb4833`; its inspected command history shows work in `.worktrees/devmap-sqlite-compatible`, whose inspected HEAD is `520683a5`. The map inventory had no working-directory report for this task. Whether an earlier report was overwritten is not established for this particular task.

Confirmed systemic defect: `replace_observed_tasks_preserving_timestamp` replaced optional location observations along with the host inventory, and the plugin explicitly instructed fallback on omission. This made reports disappear on other tasks' refreshes and process restarts.

Implemented in b714:

- Store location observations independently in repository metadata `working-directory-reports.json`, keyed by host and task ID. Preserve the original report timestamp and registered directory; do not fabricate host migration events.
- Merge explicit reports under the existing stable repository lock, with bounded checked file access and atomic replacement. Reject conflicting equal-time reports; ignore delayed older reports. Pending/corrupt stores fail explicitly instead of silently reverting locations.
- Host-only, partial/empty inventories and Git-only refreshes retain observations. A different Viewer and a restarted service read the same persisted evidence. Reports alone never recreate missing chats.
- Update both the checked-in plugin skill and MCP schema description to remove fallback-on-omission guidance.

Validation: the regression first failed on the old implementation, then passed. Complete `dock_mcp` (18) and `dock_model` (38) runs passed, plus the subsequently added damaged/pending-store test (1): **57 passed total**. Log: `location-tests.txt` (56); the additional test's successful command output is in this task. Formatting, diff check and Release build passed. One attempt to relink the model test while its earlier binary was still running hit Windows LNK1104; the serial retry after completion passed.

Installed **0.1.1+codex.20260908202531**, with matching Release/cache SHA256 **7FFB4FD70E084C17256852BE6794D7FD452874ADB459F2A7292F1A208E8550C9**. Install log: `location-install.txt`. Live processes still execute 172746, so this installed correction needs host restart before real Viewer verification. The reported SQLite task still needs an initial valid directory observation from its owner; this patch preserves reports but cannot discover never-reported execution locations from host registration alone. No claim of corrected live placement or overall visual acceptance is made. No commit/push performed.

## Live location acceptance — 2026-09-08 20:41–20:47 UTC

**Location persistence and the user's concrete misplaced-chat scenario: passed.** Overall visual acceptance remains separate and incomplete.

- After user restart, PIDs 19700 and 42520 execute `20260908202531`. The new MCP-owned Viewer on port 62754 was opened and visibly verified in the right-side Browser.
- This task checked b714 at Unix 1788900090 and reported it once. A subsequent host inventory deliberately omitted all workingDirectory fields; the task remained at b714 with the original `20:41:30Z` report. The separate repository metadata file contains that original observation.
- User explicitly authorized one message to `评估是否仅需一个数据库` requesting its own execution-directory verification and report. That task verified `.worktrees/devmap-sqlite-compatible` at Unix 1788900195 and successfully reported it through its MCP process, preserving the original host cwd.
- This task then submitted another host inventory with **no workingDirectory fields**. The SQLite task remained at `.worktrees/devmap-sqlite-compatible`, with association source `agent_reported_working_directory`, original registered project-root path, and unchanged observation time `2026-09-08T20:43:15Z`. This is real cross-task/cross-MCP evidence, not a fixture result.
- The live browser DOM contained the SQLite chat under exactly that workspace. A visible screenshot confirmed its title on the intended card at current HEAD `550e3a2a`. Evidence: `docs/audits/assets/2026-09-08-live-v2/location-fixed-live.png`.
- Observations aged to stale normally; Git refresh did not re-stamp them. The inventory remains explicitly incomplete. No fabricated location, manual metadata patch, protocol-navigation workaround, or commit/push was used.
