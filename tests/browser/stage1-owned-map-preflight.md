# Stage 1 public-map ownership preflight

This is a small Windows transport and cleanup check, not the formal performance comparison or baseline A/A calibration. It uses one cold map per executable on an existing disposable native migration fixture. Optional `clients` (1 or 4, default 1) and `samples` (1–10 per client, default 1) enable concurrent warm observations. It does not cover scale workloads or browser-ready timing.

Run with an explicit JSON configuration under the worktree's `target/verification`:

```json
{
  "schema": "devmap/stage1-map-preflight/1",
  "manifest": "ABSOLUTE_PATH_TO_DISPOSABLE_NATIVE_MANIFEST",
  "baseline": "ABSOLUTE_PATH_TO_FROZEN_A1CF_EXECUTABLE",
  "candidate": "ABSOLUTE_PATH_TO_CURRENT_CANDIDATE",
  "candidate_sha256": "EXPLICIT_64_HEX_DIGEST",
  "python": "ABSOLUTE_PATH_TO_PYTHON"
}
```

Invoke `node tests/browser/stage1-owned-map-preflight.cjs <config.json>` from the worktree. Source, manifest and both application executables must resolve inside its verification directory. This preflight is deliberately restricted to the existing two-worktree native fixture contract; it is not an alternative path admission mechanism for arbitrary repositories.

The candidate identity probe runs inside the default strict Windows Job wrapper. Before each executable's measurement, the repository endpoint must be absent; an occupied or inaccessible endpoint fails without stopping it. Each measurement worker is created suspended and assigned to a new kill-on-close Job before it can spawn MCP or runtime children. The worker starts its timer before spawning MCP, initializes it, and validates the public `devmap_read_map` response. A separate worker/Job is used for the next executable. Normal MCP close assertions remain in the worker.

The new wrapper option `--teardown-descendants-after-success` permits **planned sample teardown** only after the worker process handle is signaled with exit code zero. Job membership may still contain the on-demand owner or transient helpers. The wrapper records `cleanup_policy`, the active Job process count, and whether it terminated remaining members, then confirms the Job is empty. It always records `natural_lifecycle_acceptance: false` in this mode. An abort or worker failure cannot authorize a successful planned teardown. No PID discovery or PID-based termination is used.

Default wrapper behavior is unchanged: surviving descendants after its grace interval still fail and are cleaned up. The planned option must not be used to claim natural idle exit, absence of resource leaks, or a single owner. Those remain separate Stage 1 hard gates. The instantaneous active Job count is not a role inventory and must not be labeled “number of owners.”

Outside timing, the controller checks executable hashes/identities, source/run directory identity, every legacy file hash, complete logical SQL state, and frozen/activation backup inventories. Endpoint absence is checked again after each Job completes. Reports are retained in a fresh `stage1-owned-map-*` directory and always have `formal_acceptance: false`. The preflight currently validates the response schema via the old worker; full semantic parity is established separately by the native migration/browser checks, not inferred from matching response byte counts.

## Executed evidence

Five actual Windows controls passed in 10.863 seconds: clean default exit, strict rejection of a surviving child, explicit successful planned teardown, rejection of a failing worker, and rejection of control-pipe EOF. The initial test incorrectly expected exactly one active process despite runtime launchers/accounting; it was corrected to require a positive count and verified native empty accounting. The original failed log is retained. No lifecycle requirement was relaxed.

Actual old/new preflight `stage1-owned-map-WOxfcn` completed with empty Jobs, no errors, matching executable identities/hashes, and SQL/legacy/backup preservation. Baseline and candidate each ran one cold and one warm observation. The baseline had zero active Job processes at teardown; the candidate had two, explicitly terminated within its owned Job. The candidate's endpoint was absent after cleanup. This proves isolation, not natural descendant exit.

The single cold observations were approximately 1829.802 ms (old) and 2282.898 ms (candidate); warm observations were 575.617/60.049 ms. These exploratory observations suggest a possible cold-start regression and faster warm response, but cannot establish either percentile acceptance or representative performance. Keep the existing predeclared bounds; do not widen them from these results. The fixture already contains migrated SQL alongside preserved legacy files, so a clean matched-view scale workload and old-only calibration are still required.

Evidence: `stage1-job-policy-tests.log` (initial failed assertion), `stage1-job-policy-tests-corrected.log`, `stage1-owned-map-preflight-config.json`, `stage1-owned-map-preflight-final.log`, and the run's worker/Job/final reports under `target/verification`. Earlier run `stage1-owned-map-QtdQqh` preceded the added executable-preservation check; use WOxfcn as the final preflight evidence.

## Four-client extension

The worker now opens four MCP clients, completes each client's warmup, then releases a common gate for measured traffic. Each client's sequence, duration, response size, errors and summary remain separate; pooled timings are supplementary. Warmup failure rejects the gate, and all clients settle before reporting failure. Successful observations are retained in a failed population, including a failure during cold startup. The unchanged default is one client. This does not assert that four clients imply one owner; process role verification is still separate.

Actual run `stage1-owned-map-oeIKdP` (final worker source) completed successfully for frozen A1CF and candidate 6D26, with one cold and four clients × three measured warm maps per side. Both Jobs emptied, endpoints disappeared, and preservation passed. `stage1-owned-map-PoGmuE` is an earlier successful run before the spawn-failure cleanup correction. Neither run is formal calibration or percentile acceptance. Configuration/logs: `stage1-four-client-preflight-config.json`, `stage1-four-client-preflight-final.log`.

An actual missing-executable negative control exits with code 1 and records ENOENT plus the empty attempted population (`stage1-client-spawn-failure.json` / `.log`). The worker previously waited for an exit event after spawn had already failed; it now records that failure and closes its reader without waiting for an event that cannot arrive. This control covers startup failure, not every possible four-client mid-warmup failure.
