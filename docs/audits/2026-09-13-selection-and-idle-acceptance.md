# Selection retention and default idle resources

This closes two bounded checks under the two-stage plan, not the complete Stage 1 or original goal. The new candidate is source `1cc1831`, executable SHA-256 `4C9109976A11D1604098A7636930E781B73AA357F62D0A51E814DE2D2E3726D9`, at `target/verification/task6-candidate-1cc1831/devmap.exe`. No installed plugin, user repository or global host configuration was changed.

## Minimal UI state repair

The user-approved staged plan permits a minimal correction to the old selection-loss bug. `refreshDynamicState` rebuilt workspace cards without their existing `aria-current` marker, and appended them after map labels, changing their drawing order. The fix preserves the selected DOM object's identity and inserts replacement cards at the original position. It creates no new default selection and does not change CSS, copy, data semantics or layout calculations. The production diff from `4e426e8` consists only of five added and three removed lines in `assets/dock.html`.

The new renderer regression failed on the old implementation with an empty selection, then passed after the fix. All 104 renderer tests passed, including focus, message-count and drawing-order assertions. The relevant Rust suites passed 13 Dock UI contracts and six HTTP/SSE viewer tests. The release build completed in 31.41 seconds. These are targeted validation after a UI-only change; they are not a newly run full 690-test population.

Real Chromium fixture checks covered widths 1280, 560 and 360. The frozen old UI reproduces lost selection after age refresh and SSE; the candidate retains selection, focus, expanded cards and scroll. Pre-refresh old/new images agree, and candidate pre/post-age images agree under the existing maximum per-channel difference of two, with no excluded regions. `selection-browser-cIMlew/report.json` contains six cases and 18 screenshots; the 360px candidate after-SSE image was visually inspected. Existing clipping and layout limitations outside this state-restoration change were preserved.

The failed evidence remains: `selection-browser-3c4JHr` exposed the drawing-order defect; `selection-browser-zXG9hC` rejected 21 pixels under exact byte equality, whose maximum channel difference was two. The final run uses the already established visual tolerance, without another UI edit or region mask. The old byte-identical `sqlite-compatibility.cjs` gate remains unchanged and is not claimed passing on this intentionally changed UI; this targeted check covers the declared compatibility exception.

## Actual owner replacement

`shared-browser-ISHwtr/report.json` used the compiled 4C91 candidate with actual MCP, HTTP/SSE and Chromium. The initial owner 27068 was terminated through its retained child handle and replaced by owner 12804; MCP 27836 and the browser URL remained the same. The seven-workspace/six-task view retained selection, keyboard focus, expanded details/chats, 140% zoom and scroll (26,70). The test now requires these selection and focus assertions instead of merely reporting them. Session 28015 ended with code 0; a subsequent read-only OS check found replacement PID 12804 absent. No PID-discovered process was terminated.

This is actual runtime/browser evidence, not a real Codex in-app browser activation, automatic hook trust flow, or proof of every platform. Those first-stage host gates remain open.

## Default idle resource observation

The resource run `shared-freshness-jupjkm` measured the 6D26 executable built from `4e426e8`, before the UI-only repair. Its source and binary identities are explicit in the report. The shared-core Rust source is unchanged by `1cc1831`, so this is reusable backend-source evidence, not a claim that the 4C91 executable itself underwent another ten-minute resource run.

The existing migrated 20-worktree/100-session/100,000-event fixture passed four-client summary warmup, full cursor audit, all 14 SQL table/state checks and backup preservation. The harness deliberately terminated its four proxy children to disconnect them; their SIGTERM records are retained. It then observed owner 3196 with a retained native handle for 600 seconds, without sending owner/Git/SQL requests. The owner exited normally with code 0 after 60.808735 seconds; no harness termination was requested for the owner.

| Observation | Result | Budget / interpretation |
|---|---:|---|
| CPU during the actual alive interval | 0.025695% of one core | <1%, passed |
| CPU over the full 600-second window | 0.002604% of one core | Reported separately; exited tail cannot hide active usage |
| Sampled RSS peak | 22.102 MiB | Owner only |
| Observed lifetime RSS peak, including startup | 24.102 MiB | ≤150 MiB, passed; sampling can miss a final peak before exit |
| Natural idle exit | 60.809 s, exit code 0 | Passed default lifecycle |
| Continuous ten-minute residency | No | Expected for the default idle-exit design |
| Preservation / cleanup | Passed / no errors | SQL and immutable backups unchanged |

Resource session 98697 ended with code 0. The first UI renderer test ran at 10:42:57 UTC, after the resource owner had already exited at 10:39:14 UTC; the observer continued its zero-activity tail. Resource evidence is at `C:/Users/user/.devmap-test-fixtures/devmap-sqlite-compatible-20260909/summary-runs-96fb52d407c7462e9ecd321993bc4279/shared-freshness-jupjkm/report.json`, with configuration `target/verification/stage1-4e426e8-resources-config.json`.

The result excludes aggregate proxy/Git/helper memory and process counts; those remain separate simplification-ledger work. Formal latency/freshness, baseline warm-noise calibration, actual host setup/hooks and migration/recovery operational acceptance also remain open. No absolute performance or whole-goal completion claim is made.
