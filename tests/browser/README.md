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

`process-performance.cjs` measures native MCP full-map round trips (including
Git refresh). Set `DEVMAP_BENCHMARK_EXE` and `DEVMAP_BENCHMARK_SOURCE`; the source
must be inside this checkout's `target/verification` directory. Defaults are
20 cold processes, 10 warm-up requests and 100 timed persistent-client requests.
The `DEVMAP_BENCHMARK_COLD`, `DEVMAP_BENCHMARK_WARMUP` and
`DEVMAP_BENCHMARK_SAMPLES` overrides support smoke checks. It records executable
hash, raw latencies and response sizes. This does not measure compact internal
summaries, CPU/RSS, idle behavior or actual host integration, and concurrent
build load must be excluded from an eventual controlled acceptance run.

The ignored Rust test `performance_fixture` generates a synthetic legacy scale
corpus through domain APIs. Run
`cargo test --release --test performance_fixture -- --ignored --nocapture`
to create the default 20 worktrees, 100 sessions and 100,000 events. It retains
the disposable source under `target/verification/scale-legacy-*` and prints a
manifest path. `DEVMAP_SCALE_WORKTREES`, `DEVMAP_SCALE_SESSIONS` and
`DEVMAP_SCALE_EVENTS` set smaller smoke sizes (events is per session). Corpus
generation is not performance or real-host evidence.
