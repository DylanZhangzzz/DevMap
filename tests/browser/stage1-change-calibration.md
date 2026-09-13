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

## Completed first run: jvYk1v

Controller session 25792 exited 0. Both arms completed all 10 warmups and 100
measured changes, giving 800 measured client outcomes and no request/model
failures. All eight proxies exited normally on EOF; strict Job roots 31908 and
11672 exited 0 with empty Jobs and no aborts. Registered probe, HEADs, legacy
inventory and captured inputs were preserved; no DB was created. A separate
read-only audit of all 896 initial/warmup/measured/final response packets confirmed
20 workspaces and 100 correctly assigned sessions per response, with no truncation.
Maximum measured public full-map response was 206803 bytes in each arm.

The frozen analyzer completed, but **change noise calibration failed**:

| Metric | A1 p50 / p95, ms | A2 p50 / p95, ms | A2−A1 p95 95% interval, ms | Cap |
|---|---:|---:|---:|---:|
| Client 0 | 6011.4 / 6417.0 | 4882.1 / 5898.8 | −1194.5 to −340.9 | ±250 |
| Client 1 | 6010.5 / 6417.7 | 4931.2 / 5895.7 | −1218.8 to −352.2 | ±250 |
| Client 2 | 6004.6 / 6393.8 | 4905.0 / 5901.7 | −1085.8 to −323.8 | ±250 |
| Client 3 | 5999.6 / 6437.4 | 4897.4 / 5903.6 | −1202.3 to −334.3 | ±250 |
| Per-change maximum | 6019.5 / 6447.9 | 4969.5 / 5907.0 | −1083.8 to −345.5 | ±250 |

All five intervals exceed the allowed diagnostic range. A2 being faster does
not make this a passing calibration: both arms ran the same old executable.
It demonstrates a substantial between-arm shift in this measurement population,
not candidate speed or proof of the shift's cause. No tolerance was frozen.

A1 ran 12:35:29–12:47:40 UTC; A2 ran 12:47:41–12:58:05 UTC on 2026-09-13.
Whole-machine busy time, weighted by sampled interval duration, averaged 74.34%
during A1 and 56.59% during A2. Free memory ranged 517–1302 MiB and 403–1492 MiB
respectively. These include benchmark load and do not identify another process
or establish a paging cause. The initial OS-only intervals averaged 32.04% busy.
After controller completion, a separate 30-second OS-only window (session 21490,
exit 0) averaged 4.49%, maximum 7.04%, with at least 2019.9 MiB free. No DevMap
workload was launched during that post window. An additional instantaneous WMI
snapshot is retained without treating it as historical bottleneck evidence.

Evidence is under `target/verification/stage1-change-aa-jvYk1v`: both workers and
Job reports, raw responses, `report.json`, `analysis.json`, `response-audit.json`,
`host-summary.json`, `post-load.json` and `post-run-counters.json`. Analysis input
report SHA-256 is `e8f3ee09f813e11b56b79fa03b73aeca0c78c50ab2c86ddbd80960b9377a6bb2`.
Do not overwrite or relabel this failed calibration.

The independently observed quieter post window supports one new full A/A batch,
using the same registered input, counts, worker and analysis method. This is a
separate population after an observed environment change, not replacement of
individual samples or a retry hidden inside jvYk1v. Its own pre-run/continuous
telemetry must be retained; the post window does not guarantee future quiet.
Do not repeat indefinitely or raise the cap if the new batch still has excessive
noise. Other cold/warm and browser calibration gates remain open.
