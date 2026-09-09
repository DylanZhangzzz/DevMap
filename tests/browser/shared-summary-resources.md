# Schema-2 owned-owner resource observation

Root execution update (2026-09-09): the actual tiny fixture smoke completed its three-second native observation, exact-owner verification and full SQL/backup preservation with no cleanup errors. The owner stayed alive throughout that short window and was then explicitly terminated during cleanup; no natural-idle or 600-second acceptance is claimed. A queried lifetime RSS high-water value can miss a peak between the last alive sample and process exit, so this sampling protocol does not establish a continuously measured exact maximum.

Candidate implementation only. Node syntax/pure tests and Python AST parsing passed; no owner, native sampler, 600-second run or corpus was started in this implementation slice.

```
node tests/browser/shared-summary-resources.cjs --self-test
node tests/browser/shared-summary-resources.cjs --config C:/approved/resource-config.json
```

Configuration reuses the reviewed shared-summary receipt/candidate/python/fixture_root/run_parent contract. Acceptance fixes dimensions to 20 worktrees, 100 sessions and 100000 events, and fixes sampling to 600 seconds at 5-second intervals. Smoke permits a tiny receipt and samples for 3 seconds at 250 ms; all acceptance fields remain false. The original shared-owner-resources.cjs and prior reports are unchanged.

One explicitly owned copied owner starts with the existing 60-second idle policy. Four proxies perform ten summary warmup rounds, followed by one full baseline cursor audit and SQL/backup checks. The final identity-continuity Hello is recorded as the last request. All four proxies are closed and reaped. The Python observer then samples through a retained native process HANDLE; the first sample must still find the original owner alive. During the entire sampling window there are no Hello, ping, MCP, SQL or Git probes. Node only waits for the retained observer/owner exit events. The Python/Node preparation gap is explicitly recorded by last_request_completed, proxies_closed, idle_started and sample fields; no claim of exact zero-gap capture is made.

The owner is allowed to exit naturally. Normal exit requires Node's retained ChildProcess code=0/signal=null while in the idle phase, plus the native HANDLE's exit_code=0 and exit FILETIME. The existing helper's `stopping` marker is used only to suppress its generic unexpected-exit error during this expected lifecycle; no owner kill is requested during observation. A dedicated listener still rejects nonzero/signalled exits. Any cleanup kill after a short smoke or failure is recorded as phase=cleanup with harness_termination_requested=true and is not natural-exit evidence.

The report preserves every sample, last-alive/first-exited sample, process creation identity, native exit time and final code. FILETIME additions are decimal strings to avoid JSON integer precision loss. Existing observer fields retain their original meaning. Active duration is native exit time minus first sample time, checked against monotonic last-alive/first-exit bounds; a clock discontinuity outside tolerance fails analysis. CPU is reported separately over this active interval and the complete 600-second window. Exited samples' zero RSS/CPU tail cannot lower the active-interval gate.

Budgets follow 03-roadmap.md line 142: owner-only mean CPU strictly below 1% of one core and RSS at most 150 MiB (157286400 bytes). Both sampled peak RSS and process lifetime peak are reported; the gate conservatively uses their maximum. Lifetime peak includes startup. These numbers exclude proxies, transient Git descendants, Python/Node and the rest of the machine; no total-system or four-proxy memory claim is made.

`default_idle_resource_acceptance` requires acceptance mode, a full 600-second measurement, confirmed normal idle exit, active and full-window CPU gates, RSS gate, complete preservation and cleanup. `continuous_residency_acceptance` instead requires alive_for_entire_window=true and budgets; it is false for normal 60-second idle exit. The two interpretations are intentionally distinct. `performance_acceptance` is always false because latency/freshness/host/UI gates are separate. `completed` describes successful acquisition/analysis/preservation; a budget violation leaves explicit acceptance fields false and exits the CLI unsuccessfully while retaining the measured report.

This is stronger preservation than the older counter-only resource script: the new harness validates receipt physical identity, all 14 SQL table contents and schema, active generation, activation-backup coverage and complete immutable inventories before and after the window and after cleanup. SQLite sidecar sizes are separately recorded. Evidence is bounded to 128 MiB. No recursive deletion; only retained child objects are stopped and all cleanup outcomes are retained.

## Optional observer allocation mode

windows-process-resources.py retains its old default requirement that exe/output be below the checkout target paths. New `--owned-run-receipt run/creation.json` is explicit and compatible with a D-drive owned run. shared-summary-runtime.cjs adds candidate_sha256 to its creation receipt. The observer checks the 32-hex nonce, canonical absolute direct receipt/exe/output files within the declared run, all existing ancestors for Windows reparse flags/symlinks, executable SHA256, and exclusive output creation. It does not start or terminate the observed process.

Trust boundary: the spawning Node harness retains and rechecks the physical run-directory identity. Python does not claim to independently equate Node stat.ino/dev to Python's filesystem fields. Its independent checks are canonical/no-reparse/direct-file/build checks; the parent is trusted and this is not an adversarial atomic filesystem lock. Python then opens the process with query/synchronize/read rights, verifies its image and retains that handle throughout sampling, preventing PID reuse from changing the sampled process.

Required next evidence: independent source review; tiny owned real sampler/proxy smoke (including new D allocation mode); negative observer/exe/exit failure handling; then the separately authorized 600-second owner-only run. Pure tests do not prove native identity, cleanup or natural idle exit on this machine.
