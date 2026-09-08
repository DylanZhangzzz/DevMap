# SQLite compatibility acceptance record

Status: implementation in progress. This record does not declare the complete
simplification goal achieved. All migrated repositories below are disposable.
No installed plugin, source worktree, real repository storage or published release
was replaced.

## Frozen baseline

Branch: `codex/devmap-sqlite-compatible`, based on `db696768` plus the verified
installed frontend overlay in `520683a`. The original source checkout remains
separate. Baseline details and resource hashes are recorded in
`../superpowers/specs/2026-09-08-devmap-simplification-guide/runtime-baseline.json`.

The installed executable SHA256 used to generate old-format fixtures is
`A1CFBB1C46BD9026B18DB67DA9F70485B0BB20C6C54FD779475B52531731D419`.
The fixture harness refuses a different executable and passes a disposable
`--source` for every MCP invocation.

## Evidence and remaining gates

| Gate | Current evidence | Status / scope |
|---|---|---|
| Existing behavior baseline | 286 Rust tests, 199 JavaScript tests, 13 installed-overlay UI contracts | Baseline passed; final whole-branch rerun pending |
| Repository SQLite store | Transaction, WAL/FULL, identity/schema, integrity, backup and concurrent readers | Task1 independently approved |
| Route and binding semantics | Original starts, request identity, CAS, binding history and independent watermarks | Task2 independently approved |
| Journal and presence | Atomic acceptance/projection, retry, gaps, leases, hashes, retirement/registry identity | Task3 independently approved, including retirement fix |
| Migration | 16 migration tests and 37 related SQL tests; strict frozen sources, activation fence and drift checks | Task4 and both review fixes independently approved (`984f781` + `fab0af3`); 38 focused fix checks passed |
| Real old-format input | Frozen executable creates routes, bindings and journals across two worktrees | Passed; native process fixture, not an observed Codex host lifecycle |
| Complete model parity | Original saved native map, frozen legacy projection and SQL projection compared | Passed at one evaluation time with original task inventory metadata; only two process-local refresh counters normalized |
| Browser parity | 24 comparisons across 1280, 560 and 360 pixel widths, details, zoom/pan and accepted refresh | Passed for migrated snapshot pair; actual shared-owner restart remains pending |
| Late old writer | Actual old executable appends after activation; SQL verification diagnoses drift | Passed; SQL rows/generation, old append and frozen snapshot all retained |
| Shared owner and IPC | Same-user local transport, one owner, independent clients, reconnect | Pending |
| Automatic startup and retirement | Safe setup, absent/replaced worktree reconciliation, no read-only DB creation | Pending |
| Performance | Release scale, cold/warm latency, Git freshness, payload sizes, 10-minute idle CPU/RSS | Pending; smoke timings are not acceptance measurements |
| Actual host loop | Ephemeral Codex CLI performed map → route → map against active SQLite; structured IDs/revision/readback verified | Direct MCP smoke passed; automatic hooks and final shared-runtime host rerun pending |
| Final branch checks | Full Rust/JS, formatting, packaging and independent whole-branch review | Pending |

## Reproduction and interpretation

See `tests/browser/README.md` for the committed native fixture, migration export,
old-writer and browser commands. Generated evidence lives under the checkout's
ignored `target/verification` directory. Final Task4 native input was
`legacy-process-V2cRYf`; `native-migration-final.log`,
`native-late-writer-final.log` and `migration-browser-final/report.json` record
the corresponding checks. The fixture is deliberately divergent after the
old-writer test. Generate another fixture instead of repairing or reusing it.

The native baseline made multiple map refreshes. The migration comparison uses
an initial projection, so `revision` and `observation_revision` are explicitly
normalized to 1 for that comparison only. No domain field, warning, timestamp,
task completeness or sorting difference is excluded. This does not establish
monotonic counters across a running viewer's owner restart.

Browser resources are identical. Time and build metadata are fixed for both
sides. No screenshot region is masked. Same-source controls measured rounded
edge raster variation up to two RGB levels; the same bound applies to the
candidate, with zero pixels outside pixelmatch threshold 0.01. Raw counts remain
in the report and a significant-pixel negative control must fail. This is not
a claim of byte-identical PNG files.

## Recovery and performance boundaries

“One database” means one authoritative domain store, shared by the repository's
worktrees. SQLite WAL/SHM, operating locks, a durable activation-intent fence and
retained migration backups remain necessary operational files. The fence prevents
a missing activated DB from silently reviving older legacy data.

Source hashes detect a late legacy write; they cannot prevent an unknown old
executable from writing after cutover. Safe rollout must establish old-writer
quiescence. A process-name or PID check alone is not that evidence.

Task4 currently rejects changes to original worktree physical identities
conservatively, including legitimate retirement. Shared-runtime integration must
distinguish retirement, replacement and inaccessible state while retaining
historical origin evidence. Per-access full legacy validation and journal replay
also remain explicit performance work. Neither limitation is waived by the
passing small migration fixture.

## Shared-runtime review checkpoint

Independent review of application commit `56c927a` found
that partial inventory merges can exceed the accepted query limits after binding
writes, and that shared relationship observations can incorrectly reuse the
owner worktree's `devmap.developmentTarget` for another worktree. Both were
reproduced by tests against the old implementation, fixed in `7d10c4a`, and
independently approved. Application tests passed 9/9, including preservation of
the IPC caller's previous heads. The separately reviewed `d9c71bb` wire seam was
also approved; this does not approve the unfinished proxy integration.

Transport review fixes remove detached identity workers and correct the rejected
Hello test oracle. Commit `02ea27d` also adds full ancestor permission validation
and owner-lock retention through reactor teardown. The unsafe-grandparent case
was reproduced against the old implementation. Final focused verification passed:
11 process tests, 7 runtime unit cases (including two helper fixtures), 3 snapshot
tests, and scoped Clippy with warnings denied. Independent transport re-review
approved both fixes. Additional affected compatibility suites passed: 3 wire,
10 Git relationship and 17 migration tests, recorded in
`target/verification/task5-reviewed-compatibility.log`. A prior combined link
failed because C: was full; that run is not validation evidence.

Builds stopped when C: had no space. Generated files, fixtures and uncommitted
changes were preserved. Space later recovered to about 1 GB on C: and 3 GB on D:;
subsequent verification uses a separate D: target with incremental compilation
and debug symbols disabled. This changes the local verification environment,
not the durability, compatibility or final performance acceptance criteria.

Commit `5070f14` adds reviewed Query/AcceptInventory transport and a serial
application executor. Every physical frame remains at most 16 KiB. Complete
requests and frozen responses have separate aggregate bounds, offsets and
digests; admission reserves capacity before upload. Running and queued jobs
retain their reservations after client disconnect, and shutdown holds the owner
lock until executor completion. The 48 MiB aggregate reservation is a design
budget, not a measured RSS result.

Final focused verification passed 10 runtime cases (including two helper
fixtures), 5 query process tests and 11 transport process tests. Clippy initially
reported a large enum variant; boxing the snapshot resolved it, and Clippy,
typed duplex and actual process round-trip checks then passed. Evidence is in
`target/verification/task5-query-*-final.log`, `task5-query-clippy-boxed.log`,
`task5-query-duplex-boxed.log`, and `task5-query-roundtrip-boxed.log`.
Independent review approved this bounded slice. Public MCP/viewer routing,
mutations, reconnection, browser restart and final performance acceptance remain
separate gates. Blocking application Git/filesystem work still needs finite
operation limits; a transport deadline alone does not cancel that work.

## Public query integration and immutable domain commands

Commit `4d40a2d` introduces closed prepared mutation commands. Capture identity,
payload, receipt time and historical Git observation are fixed before dispatch;
native hook identifiers and batch normalization retain the existing rules.
Mutation-domain tests passed 7/7, existing capture tests 9/9 and hook tests 13/13.
Independent review approved this domain seam. These are domain retry tests;
production mutation IPC and abandoned-response recovery are separate work.

The executable's MCP and live Viewer now explicitly select the shared backend.
Embedded library constructors explicitly use the same RepositoryApplication
directly. Shared errors never silently fall back. MCP and Viewer retain one
ClientView, avoiding repeated inventory acceptance and pairing each SSE body
with its observation id under the same lock. Explicit supplied inventories
replace the visible subset, including an empty list; completeness remains a
coverage statement. Older or invalid input does not erase the retained view.

Focused verification passed 18 MCP compatibility tests, 6 Viewer tests, 11 map
tests and 5 proxy tests, including failed-refresh cache handling. Root also
corrected two application compatibility issues: missing registered directories
cannot establish execution location, and cached public Git facts retain their
actual observation time. Both had genuine failing regressions. Application
checks passed 9/10 initially; the remaining target-comparison oracle included
independent collectors' timestamps. Its corrected semantic comparison passed
separately, while exact retained Git time remains independently asserted.
Scoped Clippy and formatting checks passed. Logs are under
`target/verification/task5-proxy-app-final.log`,
`task5-target-comparison-time-fix.log`, and `task5-domain-proxy-clippy.log`.

The actual browser restart gate passed with an activated disposable SQLite
repository and six explicitly present fixture tasks. A persistent MCP process
and real Chromium HTTP/SSE page survived termination of the owned core and
authenticated a replacement. Task rows, original inventory time, expansion,
selection, 140% zoom and scroll were retained. Evidence:
`target/verification/shared-browser-r62msC/report.json` and
`task5-browser-restart-sqlite-final.log`. This tests state retention, not pixel
parity; the earlier frozen pixel gate is separate. Relative age labels advance
with real time and are not compared as frozen text. The original direct binary
failed the replacement gate, confirming the test detects missing shared routing.

Independent query proxy review approved this scope. Actual shared mutation
routing, safe first-write activation, legacy cutover coordination, bounded Git
operations, final large-repository resource budgets and final real-host checks
remain open. No installed runtime or live repository data was migrated here.

## Task 5: immutable mutation transport

The authenticated serial owner now accepts closed typed mutation commands.
PreparedMutation retains bounded private serialized bytes across reconnects;
revision conflicts preserve their exact message, revision and required nullable
current plan. Successful writes invalidate an existing projection without making
map initialization a prerequisite for capture. This slice adds no automatic
migration and does not yet route public MCP/hook writes through the owner.

Real copied-executable tests cover route CAS, rejected/partial uploads, anonymous
hook identity, and response abandonment after an independently observed SQLite
commit. The retained owned child is terminated and reaped, HEAD changes, and a
new authenticated owner receives the identical saved request. Complete SQL
receipts remain unchanged. This proves abandonment before caller consumption;
it does not claim the owner had not written into OS response buffers. Native
Write retains its truthful ToolCompleted plus CaptureGap pair. The initially
incorrect zero-gap test expectation and its failed log are preserved.

The initial matrix passed five cases; the corrected six-command capture matrix
passed separately in 52.28 seconds. Same-owner warm-query/write/query passed
with generation+1. Subsequent regression checks passed 12 runtime unit tests,
5 proxy tests, 5 application transport tests and 11 connection tests. After
boxing the command to satisfy Clippy, protocol roundtrip, actual owner route
dispatch and both immutable-preparation tests passed again. Scoped Clippy
passed with warnings denied. Evidence is under target/verification in the
task5-mutation-* and task5-prepared-mutation-final logs. Independent review
approved this slice; full product and performance acceptance remain open.
