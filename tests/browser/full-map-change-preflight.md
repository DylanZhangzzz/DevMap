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

## Full change-model audit and failure population

Follow-up `full-map-change-LMi2Na` (session 82041, exit 0) validates the complete
model on every converged response, including dirty states. The expected delta
changes only the target worktree's `working_state`, `relationship.dirty` and
`changed_file_count` in both lane representations, independently checked revision
fields, and already-defined generation/Git observation timestamps. Other tasks,
routes, topology, warnings, relationships, identities, and array membership/order
must remain identical to the initial model. Raw response timing ends before
whole-model hashing and disk retention. All 24 client/change responses passed;
the four measured trials form a complete population. Original data/probe/HEAD
were preserved, all proxies exited 0 on EOF, and strict Job root 16220 exited 0
with `empty_confirmed=true` and no abort.

`full-map-change-model.test.cjs` and the existing predicate test passed four
tests. Negative inputs include disappearing tasks/routes, extra lanes, changed
topology/warnings, incorrect file counts/HEAD/revisions, and stale Git timestamps.

All six trial slots are now recorded before initialization. Set
`DEVMAP_CHANGE_FAIL_AT=3` only for the deliberate preflight failure control (the
second measured trial, before its write). In `full-map-change-LOFrrS`, session
40726 exited 1 as expected. The report retains two completed warmups followed
by measured states complete/failed/not-executed/not-executed: expected 4,
attempted 2, completed 1, failed 1, not-executed 2, `full_population=false`.
The preceding dirty probe was restored, original legacy inventory and HEAD
remained intact, and all four proxies exited normally. Strict Job root 32344
exited 1, with an empty Job and no abort/cleanup error. Read-only receipt checks
confirmed this exact failure accounting; no retries replaced failed samples.
This control tests a deliberate harness interruption, not a production crash or
transport timeout. Omit the failure environment variable for normal preflights.

These results close the small-tool model-delta and interrupted-population checks,
not the required 100-change scale A/A gate. The retained legacy scale corpus has
an empty initial Git commit and no tracked file; adding a tracked probe requires
an explicitly prepared scale input before registration, not an unrecorded edit
to the frozen cold/warm calibration fixture.
