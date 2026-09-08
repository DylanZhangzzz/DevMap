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

For a generated native fixture, set `DEVMAP_NATIVE_MANIFEST` to its manifest and
run `cargo test --test migration_native_fixture export_native -- --ignored --nocapture`.
This freezes at the saved baseline evaluation time, imports, compares complete
legacy/SQL projections with the saved task inventory, exports `migration-legacy.json`
and `migration-sql.json`, then activates and verifies. It also compares against
the actual saved native baseline response, preserving inventory observation time
and completeness and normalizing only its two process-local refresh counters to
the initial projection values. Runtime restart monotonicity remains a separate
gate. Use those two paths for the browser comparison. The legacy projection uses
the unchanged shared reducer on frozen old-format inputs; it is not a second live
run of the installed binary.

Afterward, with the hash-verified `DEVMAP_BASELINE_EXE`, run
`cargo test --test migration_native_fixture native_old_writer -- --ignored --nocapture`.
It deliberately appends through the actual old MCP binary, asserts that SQL
verification diagnoses drift, and verifies both stores and the frozen snapshot
remain intact. This fixture is intentionally divergent afterward; generate a
new fixture for a later migration run. Both tests refuse sources outside this
checkout's disposable `target/verification` tree and stay ignored by default.

`windows-process-resources.py --pid PID --exe COPIED_CANDIDATE --output REPORT`
observes a verified candidate executable under this checkout's `target` directory
for 600 seconds by default. It retains a Windows process handle so PID reuse
cannot silently change the target. It records CPU as a percentage of one core,
sampled RSS, lifetime peak RSS, private bytes and whether the owner exited.
The script never starts or stops a process. `--self-test --seconds 1 --interval 0.2`
checks the observer itself; this is not candidate resource acceptance. A report
with an exited owner explicitly distinguishes zero post-exit RSS from the
resident owner's consumption. Use a new output path under `target/verification`.

`codex-host-smoke.cjs` is an explicit model-backed real-host test. Set
`DEVMAP_CANDIDATE_EXE` to a built candidate under this checkout's `target` and
`DEVMAP_CODEX_EXE` to the installed Codex CLI. It creates and migrates a new
disposable repository, then runs an ephemeral CLI session with user config
excluded and only the candidate MCP server configured for the test. It saves
actual MCP JSONL events and checks map → route → map structured results,
repository/worktree identity and route revision. It does not edit global config,
authentication or installed plugins. The child has a 180-second timeout.
Automatic hooks, desktop navigation and shared-owner restart are separate gates.
`DEVMAP_RECHECK_HOST_FIXTURE` rechecks saved events without another model run,
including a wrong-route readback negative control, into a new validated report.
