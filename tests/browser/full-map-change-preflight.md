# Public full-map change measurement preflight

`full-map-change-preflight.cjs --owned` runs frozen A1CF only, under a strict
Windows Job with a retained control pipe and a three-minute overall bound.
Set absolute `DEVMAP_BASELINE_EXE` and `DEVMAP_PYTHON_EXE`. It creates a new
disposable repository containing one tracked probe and one native SessionStart
record. It never touches the retained 20-worktree legacy calibration fixture,
starts the candidate, migrates storage, or changes host hook trust.

Four MCP clients initialize and read `devmap_read_map`. Two warmup changes and
four measured changes alternate a tracked file between dirty and clean. Timing
starts after the bounded write/fsync and ends when a parsed public full-map
response validates the expected worktree/HEAD/working state and a newer Git
observation. Raw responses are saved after timing; subsequent polling uses a
100 ms cadence with no overlapping request per client and a 30-second deadline.
This is visibility when actively requesting the public full map, not passive
browser/SSE freshness or the new compact-summary path.

The old-only verifier checks every client's observation counter advances once
per read and semantic map revision advances once per observed file change.
Initial/final full models are fingerprinted with the existing fresh-Git-time
policy; both revision fields are normalized only after these independent checks.
The clean final model must match the initial model. This old-specific revision
rule must not silently become the candidate protocol rule.

Run `full-map-change-oeTHlP` completed with no errors (session 58148, exit 0).
All 24 client/change observations converged; the 16 measured observations took
525.4–937.5 ms, each in one request. These tiny, small-repository samples are
tooling evidence, not p95 calibration or performance acceptance. Four proxies
closed normally on stdin EOF, and the strict Job confirmed empty with root exit
0 and no abort. Probe bytes, clean Git state, HEAD, and the entire original
legacy inventory were preserved; no database was created. Reports and all
response packets remain under the run directory; the adjacent `.job.json`
records process ownership/cleanup.

The predicate negative control passed: stale counters/timestamps, unknown state,
wrong worktree/HEAD, and the wrong dirty state cannot count as convergence.
Earlier attempts remain: `ced4c02e` aborted because direct shell invocation closed
the wrapper's control pipe; the Job confirmed empty. `PkqQQE` completed all
changes but failed a final fingerprint assertion because it incorrectly expected
semantic revision not to change. Its diff showed only generated/Git observation
times and the two revision fields differed; old data remained intact and proxies
exited normally. The revised explicit revision assertions passed in a fresh run.

Still required: scale input and complete-model change semantics audit, 100-change
old A/A populations with error/not-executed accounting, browser feedback/ready
calibration, and the frozen formal contract before any candidate population.
This preflight does not close those gates or relax the existing tolerance caps.
