# Baseline-only A/A calibration

The controller `stage1-baseline-calibration.cjs` accepts an explicit JSON config inside this worktree's `target/verification` with schema `devmap/stage1-baseline-calibration/1`, absolute paths `finalized` and `inventory` from the pure-legacy fixture, `baseline` pointing to frozen A1CF, and `python`. It admits only the verified 20-worktree, 100-session, 100,000-event legacy input; it never migrates or starts the candidate.

The fixed schedule is A1 then A2, each with 20 cold MCP processes and four simultaneous clients, ten warmups/client and 100 measured maps/client. Cold starts measure MCP spawn → initialize → full-map response; warm durations cover public `devmap_read_map`, response decoding and response-byte counting. Content audit follows the measured interval and affects pacing identically in both arms. Each arm runs in a separate default strict Windows Job; forced cleanup of leftovers is a failure. The 30-minute bound per arm requests owned abort and retains the attempt. Cold MCP processes close before the next sample; this old-only process boundary must not be reused as proof that candidate shared owners exit.

The full legacy inventory, physical source/run identities, absence of a database, and hashes/identities of executables, controller, worker, fingerprint helper, Job wrapper and configuration are verified between arms. The second arm must match the first arm's audited model fingerprint. The model policy validates fresh Git observation timestamps and per-client observation counters as described in `stage1-legacy-scale-preflight.md`; it does not drop business fields. Failed arms and partial samples remain in reports; the controller stops rather than silently retrying or removing them.

Run tkRaZE was dispatched with controller commit 06878bd on Windows, with the host's existing Balanced power scheme (381b4222-f694-41f0-9685-ff5bb260df2e). It uses the existing OS/filesystem caches; it does not flush caches or change power settings. No concurrent build or tests are run during calibration. Host identity, CPU model, memory at start, Node version and exact input hashes are retained in its report. This is not an assertion that unrelated user applications are idle.

## Analysis method recorded before completion

`stage1-baseline-analysis.cjs` uses nearest-rank p50/p95 and 5,000 seeded paired-block bootstrap resamples (seed 9132026). Cold samples use ordered blocks of five; warm samples use blocks of ten sequence numbers. Each warm block is resampled together across all four clients and both arms. Each client's interval is reported separately; pooled averages cannot hide it. The 95% intervals describe A2 − A1 p95 differences, compared with the existing maximum engineering caps: min(10% of A1 p95, 250 ms cold / 100 ms warm). This is a diagnostic for noise, not candidate acceptance or a newly frozen tolerance. With only two sequential arms the intervals cannot establish that future noise is bounded.

Browser loading feedback, browser-ready timing and 100-change visibility calibration remain necessary. A formal candidate contract must still be committed with explicit evidence-supported tolerances and a hash before the candidate population starts. An excessive A/A noise interval calls for improving the environment or measurement design, not widening the caps. The original absolute performance targets remain Stage 2.

## Executed result: tkRaZE

Both arms finished successfully, 2026-09-13 10:12:06–10:34:10 UTC. The retained controller session 26864 exited 0; native roots 10716 and 27156 each exited 0 and their default strict Jobs confirmed empty. All 40 cold and 800 measured warm responses passed the legacy model audit; inventory preservation and database absence passed. There were no request failures, retries, discarded observations or missing clients.

| Metric | A1 p95, ms | A2 p95, ms | A2 − A1 95% paired-block interval, ms | Maximum cap, ms | Noise within cap |
|---|---:|---:|---:|---:|---|
| Cold | 7399.774 | 7390.275 | −52.092 to 116.528 | ±250 | Yes |
| Warm client 0 | 4883.313 | 4938.587 | −77.187 to 146.450 | ±100 | No |
| Warm client 1 | 4917.935 | 4951.069 | −198.376 to 171.212 | ±100 | No |
| Warm client 2 | 4931.786 | 4937.647 | −227.844 to 145.133 | ±100 | No |
| Warm client 3 | 4867.949 | 4965.359 | −152.456 to 208.753 | ±100 | No |

The sample collection succeeded, but warm noise calibration did **not** satisfy the preregistered diagnostic. No tolerance was frozen and no formal candidate comparison began. Do not widen the 100 ms cap, discard clients, or substitute the small p95 point differences for these intervals.

Host: Windows 10.0.26200, Intel Core Ultra 7 155H, 22 logical CPUs, 16,597,598,208 bytes physical RAM, Node v24.19.0. Free RAM at start was 1,976,537,088 bytes; the post-run OS query reported 1,532,416 KiB free, with battery status 2 and 100% charge. These sparse observations suggest memory pressure deserves investigation; they do **not** establish it as the cause of latency variation. No CPU, paging or disk time series was captured, and unrelated user applications were not stopped. A later diagnostic should collect those signals at low frequency and improve conditions before repeating calibration. The existing Balanced power setting was unchanged.

Three analysis controls passed after measurement ended: zero difference for identical paired blocks, correct signed intervals and rejection for constant shifts beyond budget, and rejection of invalid/incomplete populations. The additional cold-array length assertion tightens input validation without changing the preregistered statistical method.

Evidence under `target/verification/stage1-baseline-aa-tkRaZE`: `report.json`, `A1.worker.json`, `A2.worker.json`, both stdout/stderr and native Job reports, and `analysis.json`. The analysis records input-report SHA-256 `5fa2a3efd37d10630bdd6ecbf1003b8c89ec9ba2c2cbe293fbb5ef731bb736ed`. Keep these artifacts unchanged. The original pure-legacy fixture remains unmigrated for follow-up calibration.
