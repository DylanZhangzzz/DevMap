# Owned shared-summary benchmark

Root execution update (2026-09-09): the real two-worktree/six-event smoke completed its cursor and preservation checks but failed the predeclared hot-sample cap. Candidate `64f8321` had approximately 5.65/5.63-second owner-cold observations. The failed population and all cleanup outcomes are retained; this is evidence requiring optimization, not latency acceptance. Extended local Windows paths now have actual path/junction negative tests, and SQL reads use the checked canonical path before URI construction.

Query-validation candidate update: Release SHA-256 `ed78a4dba5e928066193312899f7afbbc4058467f8c576c5b48856436b6d5bc4` completed the tiny smoke in retained run `shared-summary-S24xdI`. Cold observations were 3910/3841 ms; eight hot observations ranged approximately 197–812 ms, with all response/cursor, SQL/backup preservation and owned cleanup checks passing. Both latency gates still failed; these small populations are not scale acceptance. Earlier run `shared-summary-t48DR0` remains a failure: its second query spanned roughly 70 minutes of Windows Modern Standby, confirmed by System Kernel-Power events 506/507 (lid entry 08:35:24 local, exit 09:45:14). No failed sample was removed or replaced in that report.

Current execution update (2026-09-11): committed candidate `ad18d83`, Release SHA-256 `654816ddfc3ecff45dd0609f934733d27bb77203636da6fc956456273dc68a58`, completed tiny run `shared-summary-Q2KPRX` with request, cursor, full SQL/backup preservation and owned cleanup checks passing. Two cold observations were 3477/3368 ms; twelve hot observations had per-client p95 values 459/203/445/461 ms. Both latency gates remain false. The fixed schema-2 scale receipt exists with SHA-256 `d968690717e51b531deaeb1f095b9c615c1474543ba32c6c9bbe789756cd4620`; it contains 20 worktrees, 100 sessions and 100000 records. Its existence and separate diagnostics do not establish this harness's full scale acceptance.

This harness does not create a repository, migrate a store, delete artifacts, or invoke a model. Without arguments it prints usage. It currently supports Windows named pipes. Do not point it at the retained schema-1 corpus.

The initial implementation was checked with these syntax and pure helper commands; real execution evidence is recorded above and in the query-path performance audit:

```powershell
node --check tests/browser/shared-summary-performance.cjs
node tests/browser/shared-summary-performance.cjs --self-test
node tests/browser/shared-summary-performance.cjs --fixture-self-test
```

The fixture self-test creates only a temporary owned folder and an external-to-that-folder hard-link template, proves the writable probe rejects the link without changing the template, then removes only its exact created files/directories without recursion. The pure self-test also rejects duplicated receipt worktrees and mismatched public workspace ID sets. Those helper checks alone do not validate actual process startup, cleanup failures, cursor traversal, or performance. Any new harness behavior still needs owned fault controls for launch, timeout/response error, occupied endpoint and unexpected exit. Use a validated schema-2 receipt; an old manifest is not a substitute.

## Explicit configuration

Run only after creating and reviewing a new fixture receipt:

```powershell
node tests/browser/shared-summary-performance.cjs --config C:/approved/benchmark-config.json
```

The JSON configuration requires `mode` (`smoke` or `acceptance`), absolute `candidate` and `python` executable paths, absolute `fixture_root`, `receipt`, and an existing absolute `run_parent`. The run parent must be outside the fixture allocation. It can be a separately approved D: directory. The harness checks available space for the candidate copy plus a 256 MiB reserve, creates one new random run directory, and retains everything. This is a run-artifact check, not authorization or a capacity estimate for generating the large corpus.

Record candidate source revision/build provenance in additional configuration fields if available; the entire configuration is retained. The harness verifies the copied executable's input SHA-256 and records it, but cannot reconstruct compiler provenance from an arbitrary binary. It does not install the binary or change a running user runtime.

## New fixture receipt contract

The schema is `devmap/benchmark-fixture/1`; a scale generator may additionally write its own richer manifest. Required fields:

- `schema`, `nonce` (32 lowercase hex), `exclusive_creation: true`.
- `allocation_root`: canonical allocation path; `allocation_identity: {dev, ino}` from Node bigint filesystem stat, serialized as decimal strings.
- `schema_version: 2`, `source`, `common`, `repository_id`, `current_worktree_id`.
- `worktrees: [{root, git_dir, worktree_id}]` and `owned_directories: [{path, identity: {dev, ino}}]`. Every root/admin/source/common must have a recorded physical identity and remain inside the allocation. Worktree IDs, physical roots and physical admins must each be unique, and actual public workspace IDs must equal this exact set during the cursor audit. Declared dimensions and summary workspace totals must equal the array length. The receipt itself must be inside that allocation. Linked, junction or symlink traversal is refused using Node filesystem checks; this is not an adversarial atomic filesystem lock.
- `dimensions: {worktrees, sessions, events}`. Acceptance requires exactly 20/100/100000. Smoke must report its actual dimensions.
- `immutable_roots`: all retained frozen/backup directories, inside the allocation. An empty array is allowed only when no SQL activation backup exists. The harness reads `@activation.snapshot_path` from SQL and requires inventory coverage of each path. An external user-state backup must not be hidden or silently excluded; allocate fixture state inside the owned allocation when generating it.
- `baseline_sql` and `baseline_immutable`: generator-produced evidence using the exported `sqlState(python, db)` and `inventory(immutable_roots)` functions. Store only `{entries, sha256}` for `baseline_immutable`; the harness retains the complete before inventory in its own run. Importing the module does not run the benchmark.
- `expected_summary: {counts, totals: {workspaces, tasks, warnings}}`. Counts are the complete expected original summary object, not estimates inferred from nominal session/event totals. Source truncation must be false. Measure these once during explicit fixture setup; do not tune them to hide missing collections during acceptance.
- `change_probe: {path, sha256, worktree_id}`: one originally clean tracked file no larger than 64 KiB. The harness checks tracked membership, path and physical identity, requires exactly one hard link at both the path and opened descriptor before every write/restore, appends a small marker, restores exact bytes in `finally`, and requires dirty/clean observations with newer cycles and unchanged HEAD.

`exclusive_creation` is the generator's recorded ownership assertion, not cryptographic authentication. The operator's explicit allocation approval plus matching physical identity bounds use of this fixture; the harness cannot retrospectively prove who originally created an arbitrary receipt. Missing or mismatched receipt fields fail before owner startup.

SQL evidence uses one pinned read-only transaction, exactly 14 user tables, and hashes all rows ordered by all columns. Rows are compact ASCII-escaped JSON arrays plus LF; integers use `{"int":"decimal"}`, blobs use `{"blob":"base64"}`, and floats use Python hexadecimal strings. Schema definitions are also hashed. It preserves complete logical SQL, not physical database/WAL bytes. Sidecar existence/size is reported separately. Backup inventory includes every directory and every file's size/SHA-256 in deterministic sorted traversal. The receipt and actual immutable inventories must agree before timed work.

## Fixed populations and timing

Acceptance is fixed before execution:

- 20 separate retained owners with distinct instance nonces. Each has one newly initialized proxy and its first summary projection. Spawn-to-parsed-first-summary latency includes readiness and initialization; readiness Hello does not collect a projection. Owners and proxies are reaped between samples. OS filesystem cache remains uncontrolled. The cold gate is nearest-rank p95 ≤3000 ms, matching roadmap line 140.
- One new owner, four proxies, ten warmups each, then 100 concurrent rounds: 400 initial requests. Preserve that entire population, including refresh-affected observations.
- Extend to at most 300 total rounds to obtain at least 100 successful unchanged-`git_cycle` samples per client. Extra rows are explicitly `warm-extension`; none replace initial rows. Any measured failure fails the run and remains recorded. Insufficient hot samples at the cap fail the gate.
- Hot gate is per-client nearest-rank p95 ≤200 ms for the predeclared no-full-collection subset. All-request p50/p95/max and failures remain separately visible. Unchanged cycle is conservative source-counter evidence of no full collection; it does not prove no Git subprocesses. No instrumentation or production cache-age override is applied.

Smoke uses 2 cold owners, one warmup per client, 3 initial rounds and at most 10 rounds for 2 hot samples per client. It never reports performance acceptance.

All summary responses retain complete MCP `result` byte counts/hashes, raw bounded response evidence, outer wire bytes, and original observation fields. JSON-RPC errors without a `result` have `result_bytes: null`, not a fabricated zero-size success. The limit is 32768 bytes for every actual result, including tool error results. No harness timestamp replaces a source observation timestamp.

Correctness audits enumerate workspaces, tasks and warnings to their declared exact totals, then reconstruct detail chunks using offsets/length/SHA-256. Acceptance requires a multi-chunk Unicode detail in the generated fixture. Every page retains the original envelope; another client refreshes between capture and traversal. The small-change phase additionally checks a retained page still has its original envelope after the external file mutation. Each cursor audit has a 55-second bound and no silent restart; changed or expired cursors fail.

The separate single Git-change phase has a 15-second correctness timeout per dirty/restore detection, including page requests; fresh summaries are used for detection. This is not the ≤2-second p95 acceptance budget. The file is restored even on failure. It establishes a single dirty-file detection example only, not a percentile, four-client change visibility, or ref/HEAD-change detection. `freshness_acceptance` remains false pending the separate predeclared 100-trial cohort. SQL and backups must remain identical before and after both cohorts and final child cleanup.

## Ownership, errors and limits

Endpoint occupancy is probed before spawning. Any non-absent endpoint fails; it is never killed or replaced. Authenticated Hello must identify the exact retained child PID, build and nonce. Killing uses retained ChildProcess objects only. Every child is retained immediately, logs are bounded by 128 MiB per run, request/readiness/cleanup waits are bounded, and cleanup uses `allSettled` so one failure does not skip others. Unexpected exits and spawn/log failures fail the report. A proxy-created replacement owner would fail identity continuity and is not authorized for PID-based cleanup.

Final `completed` is written only after owned cleanup and final preservation checks. `summary_latency_acceptance` additionally requires acceptance mode and both predeclared cold/hot gates. `performance_acceptance` remains false: the 100-trial freshness gate and resource gates are not covered here. Full report/source-model equivalence, real host integration, UI acceptance, CPU/RSS/idle residency and release readiness remain separate gates. The current harness must receive real smoke process evidence before scale execution.
