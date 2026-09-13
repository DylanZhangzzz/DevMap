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

## 2026-09-13 observed result

Run `stage1-baseline-aa-prkP0X` used controller commit 50ae204. Session 91419
ended with code 0; one cold request and four sets of twenty measured requests
completed without errors. Full-model validation passed, the strict Job became
empty and the original 624-entry legacy inventory was preserved; no DB was
created. The raw report includes 32 host-load observations.

During the 159-second worker window, average machine busy time over the sampled
intervals was 75.95%, maximum 84.10%; one logical CPU reached 97.51%. Available
physical memory ranged from 503.1 to 1451.2 MiB. The one cold request took
9161.2251 ms; twenty-call per-client p95 values were 6463.2729, 6609.7690,
6438.5274 and 6423.3898 ms. These short tails do not estimate a passing contract.

After the test and its Job ended, a separate 30-second OS-only window recorded
six CPU deltas in `post-load.json` (session 23107, code 0). No DevMap workload
was launched by this task during that window. Average machine busy time was
32.56%, maximum 34.43%; free physical memory ranged from 1093.2 to 1189.9 MiB.
This demonstrates continuing host load after the benchmark, not which program
caused it and not proof of paging. No unrelated processes were stopped and no
power, priority, affinity or security settings were changed.

Decision: preserve the prior failed A/A result and do not repeat formal sampling
in this observed condition without a quieter, measured window. Do not raise
the 100 ms warm/250 ms cold-and-change caps. Future baseline and candidate runs
must share the same declared environment; record pre-run host load and obtain
paging/disk evidence if memory pressure remains a proposed explanation. The
formal candidate contract remains unfrozen. Other correctness and host work can
continue while this environment limitation remains.
