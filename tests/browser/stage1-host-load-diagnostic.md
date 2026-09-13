# Bounded old-only host-load diagnosis

Run `stage1-baseline-calibration.cjs <existing-config> --diagnostic` only to
investigate the failed A/A noise calibration. It retains the same pure legacy
20-worktree/100-session/100000-event fixture, executable hash, full-map model
checks and strict owned-Job cleanup. It runs one cold request, four clients with
three warmups and twenty measured calls each. This is one diagnostic arm, not
A/A calibration and not a formal candidate population.

Every five seconds the parent records OS CPU time counters per logical CPU and
free physical memory, without process enumeration or changing machine settings.
The telemetry is outside the tool's response timing, but may still perturb host
load. It does not record disk or paging traffic and cannot establish the cause
of latency. Low free memory is a lead, not proof of paging. Busy CPU counters do
not distinguish this benchmark from unrelated work. No build or other test is
launched by this task while the diagnostic runs.

Preserve all samples and failed output. Do not run the A/A bootstrap analyzer on
the diagnostic report or freeze/relax a contract from these twenty-call tails.
The existing A/A schedule remains 20 cold and 100 measured calls per client in
each of two arms when the diagnostic option is omitted.
