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

## Second run: 2Q4Xwp — calibration still fails

Controller 13288 exited 0; roots 36948 and 37760 exited 0 with empty strict
Jobs and no aborts. Both arms completed 100 measured changes and 10 warmups,
with 800 measured client outcomes, full model validation, unchanged registered
inputs/probe/HEADs/legacy data and no DB. All eight proxies exited 0 on EOF.
Sampling success is not a calibration pass. The unchanged frozen analyzer again
rejected all five noise intervals against ±250 ms:

| Metric | A1 p95, ms | A2 p95, ms | A2−A1 p95 95% interval, ms |
|---|---:|---:|---:|
| Client 0 | 5592.9 | 5057.2 | −1593.0 to −265.0 |
| Client 1 | 5572.7 | 5035.2 | −1601.9 to −348.7 |
| Client 2 | 5573.9 | 5066.5 | −1576.8 to −248.3 |
| Client 3 | 5535.2 | 5094.4 | −1568.9 to −263.3 |
| Per-change maximum | 5593.8 | 5108.1 | −1562.9 to −264.4 |

Evidence: `target/verification/stage1-change-aa-2Q4Xwp`, including immutable
`report.json`, `analysis.json` and derived `block-diagnostics.json`. The pre-run
window averaged 4.14% machine busy, but A1 averaged 52.99% with 434.5–2311.5 MiB
free, and A2 averaged 56.04% with 919.4–2012.9 MiB free. These are whole-machine
measurements including benchmark load, not a demonstrated external-load or
paging cause. No third blind full repeat is justified by these results. No
candidate population or final benchmark contract has been frozen.

## Bounded diagnostics after the second failure

`change-overhead-v3WXVz` replays 100 retained responses offline. Parsing, complete
model audit, serialization and individual synchronous file write/close together
had p95 7.52 ms and maximum 9.62 ms. This does not reproduce concurrent native
load or asynchronous I/O effects; it does not establish logging as the cause of
the much larger between-arm shift.

`change-resource-QSxoL3` runs the unchanged old-only scale preflight (2 warmups,
4 measured changes, four clients) with half-second owned-Job resource samples.
Controller 70807 and Job root 39292 exited 0; the Job emptied naturally, all
models and data preservation passed. Of 145 samples, 26 were incomplete. One
complete snapshot had 147 processes and 912.23 MiB summed RSS: 74 Git `cmd`
launchers, 63 native Git processes, five console hosts, four DevMap proxies and
one Node worker. Four-proxy peak summed RSS was only 57.05 MiB. RSS sums count
shared pages repeatedly, short-lived processes may be missed, and these are not
whole-lifetime peaks. The process count must not be described as shared owners.

Both installed Git paths report 2.45.1.windows.1; retained command lines show
matching `tag --points-at` invocations through `cmd/git.exe` and
`mingw64/bin/git.exe`. Upstream [git-wrapper.c](https://github.com/git-for-windows/MINGW-packages/blob/main/mingw-w64-git/git-wrapper.c)
also performs environment setup, so bypassing the launcher cannot be assumed
universally equivalent. That source is current upstream, not a verified source
match to this installed build. A bounded child-only PATH diagnostic can test this
fixture's routing and model preservation; it cannot silently alter the formal
benchmark environment, prove noise resolved, or justify a hardcoded product path.

The child-only routing diagnostic completed as `direct-git-resource-SiXuhS`:
controller 56113 and root 8308 exited 0, with no abort or remaining descendants.
The unchanged old worker used the same registered corpus and schedule. Its full
normalized baseline model and legacy inventory hashes exactly match QSxoL3;
all changed models, probe restoration and four proxy EOF exits passed. Routing
metadata records both Git executable hashes/versions, the child PATH prefix and
`where git` resolution. No parent/global PATH or product source was changed.

Across 126 snapshots (15 incomplete), no `cmd/git.exe` image was observed. The
complete-snapshot process peak was 24; maximum complete summed RSS was 246.34 MiB
and private bytes 295.60 MiB (these maxima need not coincide). This provides a
concrete routing/resource lead compared with QSxoL3, subject to the same sampling
limitations. Four measured changes took 3728–4137 ms across clients, versus
4342–4664 ms in QSxoL3. These two short sequential runs are not randomized or
paired acceptance populations; neither noise resolution nor a causal latency
benefit has been established. See `SiXuhS/comparison.json` and its adjacent
`.routing.json` / `.job.json` for retained evidence. A first launch failed a
slash-normalization assertion before spawning the workload (tool ba3f99); it
was corrected before this separate allocation and is not a discarded sample.

Next decision: preserve the ordinary installed Git routing for user-default
acceptance. Any explicitly controlled alternative environment requires its own
preregistered old/old calibration and identical old/new conditions, with the
support boundary stated. Do not patch the frozen old executable, selectively
replace observations, treat this diagnostic as a product improvement, or add
mandatory user PATH setup to satisfy the simplification gate.
