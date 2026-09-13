# Old-only scale change A/A protocol

This protocol is committed before starting the first full change A/A population.
It measures a different path from the previously failed cold/warm A/A: write and
fsync of one tracked probe to validated public `devmap_read_map` visibility.
It does not repeat the failed warm calibration, establish a quieter host, start
the candidate, or freeze a formal candidate acceptance tolerance.

`stage1-change-calibration.cjs <config>` admits the registered h7vZyC scale
receipt, frozen old A1CF, and explicit Python runtime. Config schema is
`devmap/stage1-change-calibration/1`, with absolute `receipt`, `baseline`, and
`python` paths. All configuration, input and measurement/analysis source hashes
are captured and checked between arms. The original 20-worktree / 100-session /
100000-record data remain legacy; no database or additional hook is created.

Fixed order is A1 then A2. Each independent strict Windows Job runs four old MCP
clients, 10 warmup changes and 100 measured changes, alternating dirty/clean.
The same full-model delta audit and raw-response retention used in Hs40L8 apply.
The worker has a cooperative 40-minute overall bound and 30-second per-trial
bound; the owning Job requests abort at 45 minutes. Ordinary failures restore
the tracked probe in the worker's finally block; an external forced abort is a
failed attempt and must not be assumed to have restored the probe. The controller
checks probe/legacy preservation, retains errors and does not start A2 on failure.
Both arms and every trial slot are registered before execution, including those
not executed after failure. There are no replacement samples or automatic retries.

The timing start is after write/fsync; the end validates expected worktree, HEAD,
working state and newer observation from a parsed full-map response. Whole-model
hashing and disk retention occur afterward, identically on both arms. Polling is
100 ms without overlapping requests for an individual client. This actively
requested public full-map path is not passive browser/SSE or summary freshness.

Environment: existing OS/filesystem caches and power configuration, no cache
flush, priority/affinity/security changes or unrelated process termination.
The controller records 30 seconds of pre-run OS load and five-second CPU/free-RAM
observations during both arms. No concurrent build/test/browser work will be
launched by this task. Unrelated user load may remain; telemetry does not identify
its cause or prove paging. This first change A/A characterizes its own noise;
the prior approximately 33% background busy observation remains relevant.

`stage1-change-analysis.cjs` fixes nearest-rank p50/p95, 5000 bootstrap replicates,
seed 9132026, 95% intervals and ordered blocks of 10 trials. Four clients share
each resampled block across A1/A2; per-client results and the per-trial maximum
are reported separately. Compare the A2−A1 p95 interval with the existing cap
±min(10% of A1 p95, 250 ms). These are noise diagnostics, not candidate thresholds
or evidence that a delay is imperceptible. Incomplete/failed populations cannot
produce a passing analysis. Two sequential arms cannot bound future host noise.

Seven schedule/model/predicate/population/analysis controls passed before launch,
including a constant 300 ms shift rejected against the 250 ms cap. If noise
exceeds the cap, retain the result and improve conditions; never widen the cap.
Browser feedback/ready timing, cold/warm calibration on the consistently chosen
formal corpus, candidate contract registration and formal old/new comparisons
remain separate open gates.
