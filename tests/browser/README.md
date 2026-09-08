# SQLite frontend compatibility gate

Run `node tests/browser/sqlite-compatibility.cjs` with Playwright, pngjs and
pixelmatch available under `CODEX_DOC_MODULES`, and Chrome at
`CODEX_DOC_BROWSER`. Defaults match the current Windows verification host.

The reference is commit `520683a`, whose frontend was matched to the installed
DevMap binary. The test first requires identical assembled frontend resources.
It then renders baseline, a second baseline control, and candidate at three
viewport sizes. It compares exact extracted interaction state and full-screen
PNG images after expansion, inspection, zoom/pan and an accepted SSE update.
No screen region is masked. Runtime version/build text and the clock are fixed
for both sides. Timers use the Playwright clock API.

Repeated rendering of the same resources showed rounded-edge raster noise of
at most two RGB levels. Both the control and candidate must have zero pixels
outside pixelmatch threshold 0.01 and maximum channel delta <= 2. The report
retains raw differing-pixel counts: this is not a claim of byte-identical PNGs.
A deliberate pixel mutation verifies that the perceptual comparator fails on
a significant difference.

Outputs are under `target/verification/sqlite-browser/`. A default fixture run
is **only a frontend gate**. To compare backend output, provide both
`DEVMAP_BASELINE_SNAPSHOT` and `DEVMAP_CANDIDATE_SNAPSHOT` JSON paths. Migration,
domain semantics, live host transport and performance require their separate
acceptance checks; the fixture pass does not establish them.

`DEVMAP_BASELINE_EXE=<verified binary> node tests/browser/create-legacy-fixture.cjs`
creates a separate repository and linked worktree under `target/verification`.
It verifies the baseline executable hash and uses real MCP processes to write
route revisions, bindings and events in the old format. It saves the complete
request/response exchanges, inventory, snapshot and source locations, plus
frozen copies of both legacy storage roots with per-file hashes. The
script never runs the binary against the working repository. These disposable
sources are available for the later migration test; generating them alone is
not a migration pass.
