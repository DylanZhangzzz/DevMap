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
| Browser parity | 24 comparisons across 1280, 560 and 360 pixel widths; actual shared-owner replacement preserves browser interaction state | Migrated snapshot and earlier shared candidate passed; final candidate rerun pending |
| Late old writer | Actual old executable appends after activation; SQL verification diagnoses drift | Passed; SQL rows/generation, old append and frozen snapshot all retained |
| Shared owner and IPC | Same-user transport, authenticated owner/reconnect, immutable write retry, 12 simultaneous real MCP writers | Reviewed and passed through `425f6af`; final branch rerun pending |
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

## Single-pass SQL journal summaries

Cold summary reads now validate SQL journal rows once without reconstructing a
full NDJSON buffer and decoding every event a second time. Full-record readers
and the frozen migration parser remain unchanged. The summary path preserves
registration/origin, canonical encoding, hashes, row and event identity,
contiguous sequence, unique IDs, previous links, byte/count bounds and saved
accepted extent. Indexed event IDs and record JSON are bounded before owned
string allocation. Existing transaction pinning and data_version invalidation
remain in place, including externally modified rows with unchanged generation.

Five differential/corruption tests passed in 0.03 seconds
(task6-stream-summary-final-green.log). Prior snapshot/tamper/activation tests
also passed in the integrated 50-test library run. Independent review approved
the corrected source; all-target Clippy passed with warnings denied
(task5-integrated-clippy-verified.log). Two earlier fixture changes attempted to
delete referenced SQL rows and were rejected by foreign keys; the corrected
fixtures alter metadata while preserving those constraints. Failure logs are
retained. The independent review also prompted the indexed-ID allocation bound.

An earlier read-only component profile on the preserved 100,000-event fixture
measured 50.845 seconds for the old cold SQL path and 22.879 milliseconds with
its summary cache warm, in a debug build. Those are diagnostic baseline values,
not optimized release acceptance. This change has not yet established the cold
open, hot summary or RSS budgets; metadata and SQLite engine allocations are
not a blanket bounded-memory guarantee.

## Shared public writes and bounded Git execution

Public MCP semantic/route writes and the CLI native hook now use immutable
prepared commands through the authenticated shared owner. Embedded direct APIs
retain their explicit legacy-compatible policy. Proxy identity binds to one
canonical repository/worktree; reconnects reuse the same prepared command and
preserve SHA receipts, route conflicts and anonymous invocation identity.
Admission retries share an absolute 25-second budget with bounded backoff and
one transport reconnect. Accepted exchanges retain their existing completion
semantics; this is not cancellation of accepted writes.

Read-only Git commands have a shared operation deadline, four-child admission,
per-command limits and independently drained bounded output. Owned Windows Job
Objects retain process-tree cleanup responsibility. Unconfirmed cleanup poisons
the owner and prevents even queued work from executing while retaining its lock.
Unix process-group containment is implemented but has not been executed here.
A thread-local Tokio reactor experiment hung during thread teardown; the final
runner explicitly drops its per-command reactor before returning. The owned
fixture failure logs and behavioral quarantine RED remain available.

Topology ref inspection batches commit type information and resolves unusual
tag chains by captured immutable OID. The original 256-ref cap, missing-object
boundaries and no-lazy-fetch behavior remain covered. A partial-clone missing
annotated-tag target and a nested-tag case are included in 17 passing tests.

Current integrated evidence in target/verification includes 50 library tests
(3 ignored), 8 bounded-Git tests, the Windows quarantine test, 3 public mutation
process tests, 4 retry tests and 17 topology tests. The final
task5-final-domain-owned-candidate.log passes all 12 MCP stdio, 10 repository
application and 4 worktree inventory tests. Twelve simultaneous real MCP
processes preserve all records, contiguous sequence, unique opaque IDs and
the exact actor/locator/quotation set. Earlier Busy failures and two corrected
test-oracle mistakes remain recorded. Each real MCP test executable is now
copied into its owned fixture so an idle shared owner cannot pin Cargo's output.

Existing dock/map/hook/relationship/inspector regressions passed in the retained
task5-integrated-compat logs. JavaScript tests pass 199/199; both frozen frontend
asset SHA256s are unchanged. All-target Clippy passes with warnings denied;
format and diff checks pass. Independent reviews approved the mutation adapter,
bounded runner, topology correction and admission retry. Fresh automatic SQLite
startup, final optimized performance/resource measurements and whole-branch
browser/host acceptance remain open; these slice results do not close the goal.

## Fresh shared-write startup and cooperative transition

The first valid shared mutation or inventory acceptance now activates SQLite
only after proving that every legacy origin is strictly empty. Query/Hello and
invalid input do not create a database, backup, fence or transition directory.
Existing legacy artifacts retain legacy authority. Candidate domain writes,
legacy journal/presence directory creation and explicit maintenance transitions
share one private transition guard. Internal guarded operations pass that token
explicitly rather than recursively taking the same lock.

Capture input and complete envelope validation now precede storage setup. The
same payload builders remain authoritative in CaptureKernel and the new
preflight. Inventory validates its fully merged future state and binding tuples
before setup. Route input/ref syntax is prevalidated; current route state and CAS
remain transaction-authoritative.

Fresh activation retains an external owned attempt, empty frozen manifest,
shadow provenance and durable activation fence. Automatic shadow recovery
requires the exact owned manifest and an exact permitted table inventory;
generation zero alone is insufficient. Independent review identified and closed
an extra-registry/provenance-row hole. Incomplete or unowned state is retained
for recovery. A verified pre-attempt backup PermissionDenied can preserve legacy;
errors after activation do not fall back.

Windows TEMP on this machine grants sandbox accounts Modify/DeleteChild. The
first integrated startup run correctly rejected those parents (0/7, retained
task5-startup-first-implementation.log). The same compiled binary passed 7/7 in
78.65 seconds under a trusted fixture parent, without weakening ACL checks.
Subsequent tests use TMP/TEMP and durable state roots scoped to the checkout's
verification fixtures. Conventional Windows AppData is separately unsuitable
for the private default backup here; default selection uses the validated user
profile's .devmap-state directory. Redirected LOCALAPPDATA remains strict.
The actual default selection was verified read-only, without creating user state.

Final production-source evidence: 9 startup storage tests passed in 13.35 seconds
and 4 pure validation tests in 0.16 seconds. The integrated regression log passes
13 hook, 11 presence, 12 SQL journal, 17 migration and 7 public startup tests;
the public group took 82.08 seconds. Four simultaneous first writes preserve all
four exact receipts, one activation and generation four. A separate actual
Windows PermissionDenied test passed in 9.22 seconds after correcting a test
helper's PowerShell module-loading dependency. It confirms ACL restoration,
exact event content, canonical SHA/hash chain, identical retry and continued
legacy writes without DB/fence/attempt creation. The failed helper log remains.

These tests do not yet prove every process-termination point in automatic
startup or native Unix behavior. Worktree deletion/replacement is also a known
open acceptance issue: a new real removal test reproduces the current global
origin-drift read failure in 8.50 seconds while SQL and backups remain intact.
Its tests-only file belongs to the next slice, not this startup implementation.

## First optimized scale profile

An optimized release binary built successfully in 52.11 seconds. Its SHA256 is
4F0B592583CC6B25C3164B4D6BC476F225ECCC33E25D32FA448A5D2BA0A58B56.
The preserved 20-worktree/100-session/100000-event fixture remained at generation
zero. A single read-only component profile measured source resolution 739 ms,
store open 22 ms, full legacy drift validation 3145 ms, cold SQL inputs 2192 ms,
cached SQL inputs 1.86 ms, full Git collection 8058 ms, pure projection 13 ms and
serialization 0.23 ms. Output was 206917 bytes.

This is bottleneck attribution, not a p95 acceptance run. Cached SQL input time
is not end-to-end hot query latency, and this full map is not a compact summary.
The cold-open, hot-summary, default-payload and idle-resource gates remain open.

## Narrow Git command reduction and startup recovery states

Commit following 8ef9a4a batches the three workspace path rev-parse commands
into one, with original independent calls retained for ambiguous newline paths.
It also reuses the already measured behind count for fork distance only when
merge-base equals the worktree HEAD and ahead is zero. Divergent and real
criss-cross histories retain their explicit distance query. Git relationship
regressions passed 12/12 in 12.03 seconds; the inspector passed 4/4 in 1.26 seconds.
Native Unix newline-directory coverage is present but was not executed here.
A new release/process-count measurement is still required before claiming speedup.

Four durable startup-state tests passed 4/4 in 4.82 seconds and an environment
control passed 4/4 in 5.58 seconds: complete pristine/imported/fenced owned
snapshots resume to active generation zero with exact retry preservation;
missing database or partial snapshot is refused without deleting evidence.
These construct persisted states; they are not kill-at-fsync fault injection.

Verification now uses a trusted temporary fixture directory outside Git. The
previous trusted directory inside this checkout caused Git to discover its
ancestor repository in an existing non-repository test. The same already-built
inspector executable passed after moving only its temporary fixture root. No
production repository-discovery behavior or system ACL was changed.

Lifecycle qualification remains under development: initial removal and remaining
source tamper cases pass, but foreign .git redirection exposed a real missing
identity check, and same-path new-session writes remain intentionally blocked
by the strict write gate. These open cases are not included in this scoped slice.

## Qualified history and bounded summaries — partial acceptance

The deleted-owner-anchor regression now passes: a live client can reanchor the
same repository application only when its previous root is confirmed absent.
Foreign Git pointers, wrong administration backlinks and altered surviving
legacy bytes still fail. Latest qualified-origin suite passes 7/7 in 43.03 s,
including same-path replacement with a fresh no-route journal+presence capture.
Old sessions and opened handles cannot inherit the replacement. Historical SQL
rows and frozen provenance remain retained. Worktree moves are still open.

Public MCP map regressions pass 11/11 in 52.13 s and stdio passes 12/12 in
24.02 s, including twelve simultaneous captures. A separate direct SQL journal
concurrency test still times out at the backend transition lock (11/12 passed).
Its remaining fix is being investigated without increasing the production
timeout or weakening integrity validation. This is not full concurrency acceptance.

The explicit `view=summary` path passes three public direct-MCP tests in 12.28 s
and three UTF-8/packing/detail/replay/expiry unit tests in 0.01 s. Every MCP result
is bounded to 32768 bytes, excluding the caller-owned JSON-RPC envelope. Cursor
pages retain original observation times and warning coverage. The default map
and frontend remain unchanged. Actual shared-process summary latency and byte
checks remain pending; retained-page speed is not fresh-query performance.

Startup regression passes 8/8 in 17.90 s. The Windows permission fixture now
round-trips its own unchanged access descriptor before saving its deny-injection
baseline: Windows otherwise adds only the AUTO_INHERITED flag during restoration.
Exact SDDL comparison remains; no product, parent or system ACL was relaxed.

Two native-origin lifecycle regressions remain RED: worktrees created after
activation can still inherit old route/binding attachments when their path is
reused. The next schema-2 slice will record route-revision and binding origin
identity independently of journals, with an internal current-binding cursor.
Existing public history must not be rewritten or supplied fabricated migration
events. The unreleased schema-1 performance corpus will remain untouched.

## First-release ten-minute resource observation — limited evidence

`target/verification/shared-resources-Usi5DH` retains the executed harness,
report and native-handle samples for the first release hash above. Four full-map
warmups took 12372.74, 10755.74, 10845.44 and 11175.04 ms. No requests followed
for 600 seconds. The owner was last observed alive at 60 s and exited at 65 s;
sampled peak RSS was 22.19 MiB and lifetime peak was 25.625 MiB. Whole-window CPU
was 0.002604% of one core; the conservative active-interval bound was 0.026042%.

That harness did not record the owner's exit code, so successful natural idle
shutdown is unproven. It compared generation/backend/repository/session/event
counters only; it did not prove every SQL, Git or frozen-backup byte unchanged.
The revised, independently reviewed harness records and checks exit status,
separates still-alive results, saves its own source/hash and narrows preservation
claims. Syntax validation passed; a final-candidate ten-minute run is pending.

## Journal contention and partial-provenance follow-up

The direct SQL journal concurrency regression now passes with the same timeout.
Admission supplies its already verified target identity to append/replay;
native stores without activation provenance use the existing strict reciprocal
target checks, while migrated stores retain full frozen-origin validation.
The first regression passed journal 13/13 in 6.28 s, lifecycle 7/7 in 41.43 s
and worktree inventory 4/4 in 1.60 s. New negative cases cover pointer changes
before acceptance and during the callback; the latter checks all SQL tables
for rollback, including registry, session, head and presence projection contents.

Independent review found a partial-provenance gap: deleting both the activation
row and fence could incorrectly select native admission while per-source
migration evidence remained. A real migrated fixture reproduced it in 10.70 s.
The fix rejects any remaining migration source in that state. The same test
then passed in 9.74 s, with no callback invocation, unchanged eleven-table
snapshot and unchanged frozen backup. Independent final source review approved.
Final journal 13/13 and migration 17/17 regressions passed in 6.27 s and 26.24 s;
all-target Clippy passed with warnings denied in 6.50 s. Native route/binding
identity and end-to-end performance remain separate open gates. The reviewed
[next implementation plan](../superpowers/plans/2026-09-09-devmap-native-origin-identity.md)
preserves unknown watermark-only associations, old receipt/CAS behavior,
absent-worktree intent edits and explicit retargeting.

## Native migration and browser refresh at bfa0f11

Fresh disposable fixture `target/verification/legacy-process-Oz0LW7` was generated through the frozen native executable (SHA-256 `A1CFBB1C46BD9026B18DB67DA9F70485B0BB20C6C54FD779475B52531731D419`). It contains main/linked worktrees, actual route revisions, task bindings and journal events; no fixture SQL was hand-authored. `export_native_migration_pair` passed at `bfa0f11` (one test, 3.65 s, build 18.03 s): the complete frozen legacy and SQL models are equal and match the saved old-process model after normalizing only its two process-local transport counters. Activation and verification succeeded.

The actual Chromium comparison used these exported backend snapshots at widths 1280, 560 and 360. All 24 control/candidate comparisons passed over initial rendering, workspace details, zoom/pan and refresh. There were zero visible differing pixels, no excluded pixels, and a passing negative control. Small desktop raster differences stayed within the existing two-channel-level tolerance. Clock and build metadata were fixed; assembled frontend resource SHA-256 remained `1F2BF0BEA4844BF6E6E59A57AF3DD7DC1C62507F0F7E66F8C87EBFE007405C36`. Root also inspected the sidebar details images. Report and images: `target/verification/task6-bfa0f11-native-browser/`. This is real rendering of exported native/SQL data, not simultaneous live old/new backend lifecycle coverage.

The separate native late-writer test then passed (one test, 0.86 s): the frozen old executable appended 1043 bytes after activation, verification reported legacy inventory/hash drift, and the tested SQL backend state/generation/journal contents and frozen journal bytes remained unchanged. The appended legacy bytes were preserved. The fixture is now intentionally divergent and must not be repaired or reused for normal migration success tests. See its `late-writer-report.json`; command logs use `target/verification/task6-bfa0f11-native-` prefixes.

The previously rebuilt release CLI at `bfa0f11` has SHA-256 `5C992E17273AD326F8BAE8139191A88B7138F575274A9C3A9E466FEF20220DC3`. Its separate real HTTP/SSE/MCP owner-restart run, `shared-browser-4DiLLh`, passed with seven workspaces and six tasks: owner PID 6840 changed to 25356 while MCP PID 2772, browser URL, accepted task rows/timestamps, expanded cards, 140% zoom and scroll position remained. Root viewed before/after screenshots. Selected `aria-current` changed from one item to none; keyboard focus remained on the map tools control. This result does not prove selection retention or pixel parity. The replacement owner subsequently exited, confirmed by a fresh process lookup before the next experiment. Actual host lifecycle and final full live old/new comparison remain separate open gates.

## Frontend source checks after query cohort integration

At `f701e4e`, SHA-256 checks of both `assets/dock.html` and `assets/metro-core.js` matched the exact values in the frozen `runtime-baseline.json`. Evidence is retained in `target/verification/task6-f701e4e-frontend-hashes.json`. The current `node --test tests/metro_core.cjs tests/dock_renderer.cjs` run exited 0 with **199 passed, 0 failed, 0 skipped** in 2006.30 ms; its full output is in `target/verification/task6-f701e4e-js.log`.

These checks establish unchanged frozen frontend source bytes and current core/renderer logic tests. They do not substitute for current live old/new browser behavior, runtime reconnect state or host integration. The separate all-target Rust run is still pending at this checkpoint and is not counted as passed here. No installed resources were changed.

## Current frontend regression after configuration and directory optimizations

At 1975f57 (production 1c03505), both frontend files still match their frozen SHA-256 values: dock.html CB30C346129F6BA15670D0A6419D2CCBD7F939F4A7B1C86FED1359B0825F3FC0; metro-core.js 2D1C0968BDA36F9EFE58CCB7AB80A448CD519B87DE7F8DBAE1CF6E079E0C37AD. Node core/renderer regression exited 0 with 199 passed, zero failed/cancelled/skipped in 1959.13 ms. Evidence: target/verification/task6-1975f57-js.log and task6-1975f57-frontend-hashes.json. This refreshes the source/logic evidence only; current live old/new browser and host lifecycle gates remain open. The current all-target Rust root is still running at the time of this entry.

## Full current Rust regression after directory witness optimization

The all-target Cargo root 51559 has now terminated with exit code 0. The command was cargo test --all-targets --no-fail-fast -j 2 -- --test-threads=1, using the isolated test state directories and existing D-drive build target. The compiled source was 1975f57 (production 1c03505); subsequent frontend audit edits changed documentation only, and the relevant source/test/assets/Cargo/build inputs were compared against that revision.

All 58 Cargo test groups have successful final summaries: **659 passed, 0 failed, 10 ignored**. The core library contributed 227 passed and six ignored; integration groups covered adapter protocols, Dock/MCP semantics, Git relationships/topology, shared owner/query/mutation behavior, journal integrity, SQLite migration/activation, identity replacement, worktree moves, retries and rollback. The summary uses only each Cargo group's final result, so nested helper-process summaries are not counted twice. Compilation took 40.23 s; reported suite durations sum to 1532.21 s, which is not a separately measured whole-command wall time.

The ten ignored entries are six explicitly invoked library fixtures/diagnostics, two native old-version migration fixture tests, and two performance-fixture generators. They require their own owned runs. All-target testing does not include doctests, and the debug long-session performance test is not evidence of the release latency gate. Current full live old/new browser behavior, host lifecycle, resource measurements and formal latency populations remain open. No installed plugin or user repository data was migrated.

Evidence: target/verification/task6-1975f57-full-rust.log, task6-1975f57-full-rust-summary.json, and the separately observed terminal root exit. Existing helper task6-summarize-full-rust.cjs was invoked with the explicit revision, and every group was checked for a nonmissing successful final summary.

## Fresh native migration and current browser evidence

At source b569d53 (production 1c03505), a new owned fixture legacy-process-bGXYcw was generated through the hash-verified frozen A1CFBB1C executable. It contains two real worktrees, route revision two, binding records and events, and its complete exchanges/inventory/frozen files are retained. Existing divergent fixtures were not reused. The current native migration/export test passed in 3.69 s after an 11.51 s build (root 99860, exit 0), comparing the actual saved native response, complete frozen legacy projection and SQLite projection with only the two previously documented transport counters normalized.

Current Chromium rendering used those exported projections at 1280, 560 and 360 pixels. Root 75960 exited 0; all 24 control/candidate comparisons had zero visible differing pixels and zero exclusions, maximum channel delta two, and the negative control passed. Clock/build metadata were fixed and animation disabled consistently. The assembled resource SHA remains 1F2BF0BEA4844BF6E6E59A57AF3DD7DC1C62507F0F7E66F8C87EBFE007405C36. Root inspected the sidebar details image. This is current backend-snapshot rendering evidence; it is not a simultaneous live old/new lifecycle comparison.

The separate native late-writer test passed in 0.82 s, root exit 0. The frozen old executable appended 1043 bytes; verification detected legacy inventory/hash drift, the SQL state and frozen snapshot stayed unchanged, and the appended legacy data was retained. The bGXYcw fixture is now deliberately divergent and must not be repaired or reused for ordinary migration success tests. Evidence: target/verification/task6-b569d53-native-{fixture,export,browser,late-writer}.log; legacy-process-bGXYcw manifest/exchanges/migration projections/late-writer-report.json; task6-b569d53-native-browser/report.json and screenshots.

A fresh actual Release MCP/HTTP/SSE/Chromium owner replacement also completed (root 80076, exit 0), fixture shared-browser-NM1RyV, candidate SHA 940C50692C647E21C7C389BCCF270CEC895BB63C584020903FAA185087F7531A. Owner 21660 changed to 22368 while persistent MCP 22820 retained the same URL, seven workspaces, six accepted task rows/timestamps, expansion, 140 percent zoom and scroll position 26/70. No JavaScript errors were reported. The selected workspace aria-current marker changed from one item to none, while keyboard focus stayed on map-tools-trigger. Root inspected before/after screenshots and confirmed the missing outline, so selection retention and full pixel parity remain unproven. The replacement owner was observed with the exact owned executable/instance and subsequently confirmed exited by a fresh process lookup. Evidence: task6-b569d53-owner-restart.log, task6-b569d53-replacement-owner-exit.json and shared-browser-NM1RyV report/screenshots. No user browser tab, installed plugin or original repository data was changed.

## Selection/focus attribution against the frozen frontend

The browser comparison now includes aria-current object identities and keyboard focus in its existing per-state deep-equality assertions, and retains all interaction states in report.json. Production assets are unchanged. The strengthened test ran successfully (root 28677, exit 0) using the previously exported legacy/SQLite JSON pair from bGXYcw; it did not re-import or read that now-divergent live store. All 24 pixel comparisons and the negative control still passed, with no excluded pixels.

At each width 1280/560/360, the frozen 520683a frontend, repeated control and current frontend all had one selected object at zoom-pan and zero after refresh-retained. All nine paths retained focus on map-tools-trigger. Thus selection/focus behavior matches the frozen reference for this controlled refresh; the disappearing outline is not a new difference in this comparison. Source inspection explains the shared behavior: refreshDynamicState removes route-platform nodes and rebuilds them with renderPlatforms (assets/dock.html around lines 1798-1800), without restoring aria-current; setOnline and the age timer invoke this path. The earlier renderSnapshot selection restoration therefore does not guarantee retention through the subsequent dynamic refresh.

This classifies the observed outline loss as existing frontend behavior for these inputs. It does not claim selection retention, a live native old/new reconnect pair, or complete user-invisible switching. The frozen UI remains unchanged. Evidence: target/verification/task6-selection-control-browser/report.json (interaction_reports), screenshots and task6-selection-control-browser.log. The harness change makes future selection or focus differences fail rather than relying only on screenshot comparison.

### Complete current Rust regression at 211cae3

The full `cargo test --all-targets --no-fail-fast -j 2 -- --test-threads=1` process (root 16066) terminated with exit 0. All 58 Cargo groups had final successful summaries: 665 passed, zero failed, ten intentionally ignored. The library group passed 233 with six ignored in 331.79 s; the full build took 42.25 s. Counting uses each group's final summary and does not double-count owned child fixture output. The remaining four ignored tests are two explicit native-old-binary migration cases and two performance fixture generators. Debug long-session performance's early return is not release performance evidence; no Doc-tests were run by this all-target command.

This refresh covers production 211cae3, including bounded SQL parsing and shared proof directory capture, and supersedes the older 659-pass regression for those changes. It does not complete the native fixture, actual browser, current host/resource, formal latency or final retirement audit gates. Current CLI smoke still fails cold and hot thresholds. Evidence: target/verification/task6-211cae3-full-rust.log and task6-211cae3-full-rust-summary.json; the matching root exit was verified independently of the log parser.

### Fresh native migration and browser pair at production 211cae3

A new isolated fixture, target/verification/legacy-process-z70bpm, was created through the verified frozen A1CFBB1C legacy executable. The current compiled native-fixture test binary (SHA-256 593FAA0BFEF4C023CC46C8DEF675D4CB52FD141DD3F19B8D89B4CC77D1294109) passed export_native_migration_pair in 3.65 s with exit 0, comparing real old output, frozen legacy projection and migrated SQL projection. This is a native executable fixture, not a real Codex host lifecycle.

Real Chromium snapshot-pair verification then exited 0 (root 3374): 24 frozen/control/candidate comparisons across 1280, 560 and 360 widths had zero visible differing pixels, zero excluded pixels and a passing negative control, with the established maximum channel delta of two. Time/build metadata were fixed and animations disabled. Interaction comparisons included selection and focus, details, zoom/pan and refresh. The existing frozen refresh behavior still clears the selected marker; parity does not claim absolute marker retention. The candidate 560-pixel details view was visually inspected. Assembled resource hash remained 1F2BF0BEA4844BF6E6E59A57AF3DD7DC1C62507F0F7E66F8C87EBFE007405C36. This proves rendered frozen-input compatibility using newly exported backend data; it does not prove real live reconnect or current host/resource acceptance.

The separate real old-writer test then passed in 0.82 s, exit 0. The frozen binary appended 1043 bytes after activation; current verification detected legacy source inventory/hash drift, while SQL and frozen snapshot stayed unchanged and the appended legacy data was retained. z70bpm is now intentionally divergent and must not be repaired or reused as a clean migration source. Evidence: target/verification/task6-211cae3-native-{fixture,export,browser,late-writer}.log, task6-211cae3-native-browser/report.json and screenshots, and legacy-process-z70bpm/late-writer-report.json. These two native fixture passes supplement the two explicitly ignored cases in the completed all-target run; they are not added to its 665-pass count.

### Actual owner restart refreshed at production 211cae3

Current release CLI A94ADB01A0D83DC397D04EA94D0A4892E63C4C60878EFE2B38AFC6A8829CCB10 passed the real Windows MCP + HTTP/SSE + Chromium owner-replacement harness (root 26395 exit 0), fresh owned fixture target/verification/shared-browser-gt0Ip0. MCP PID 28228 stayed connected while owner 36640 was replaced by authenticated instance 2cd9721b26d6ff40e2fc99ef0e2ff774 / PID 25188. The browser retained the same URL, seven workspaces, six accepted task rows and original observation timestamps, expanded details/cards/more, 140% zoom and scroll (26,70); JavaScript error collection was empty.

Before/after screenshots were visually inspected. The selected aria-current marker changed from one to zero while keyboard focus remained on map-tools-trigger, matching the previously verified frozen frontend refresh behavior. Passing this test proves its stated data and UI-state retention assertions, not absolute selection retention, full pixel equality or a matched live old/new reconnect comparison. Observation age naturally advanced from 2 to 6 seconds.

After harness completion, the replacement process was separately checked against the exact owned executable path and instance, then observed exiting through its process handle without a termination request. Exit observation completed at 2026-09-13T02:03:28.7081763Z (wait root 42658 exit 0). No known owned browser/owner process remains from this run. Evidence: target/verification/task6-211cae3-owner-restart.log, shared-browser-gt0Ip0/report.json and before/after.png, plus task6-211cae3-replacement-owner-{observation,exit}.json. Formal latency, current host/resource and final acceptance gates remain open.

### Current source 9360743 native migration and frozen browser pair

A fresh disposable legacy-process-rv4TSJ fixture was generated through the verified frozen real executable A1CFBB1C46BD9026B18DB67DA9F70485B0BB20C6C54FD779475B52531731D419. The current all-target build produced migration_native_fixture-0aebb95b1cac3cbc.exe SHA-256 CB26CA855232A520BCE324F661BEF3B44A9F42E9DD9E84D6A1583ABD51B52A40. Explicit export root 2740 exited 0, one test passed in 3.85 s: frozen legacy and SQL full models matched; the original real process snapshot matched after normalizing only its two process-local revision counters. Migration was activated and verified in this owned fixture.

Browser root 16909 exited 0 using this native model pair. At widths 1280, 560 and 360, initial/details/zoom-pan/refresh states produced 24 control/candidate comparisons with zero visible different pixels, zero excluded pixels, maximum channel delta 2 within the fixed tolerance, and a passing negative control. Clock/build metadata were fixed and animations disabled. Resource SHA-256 1F2BF0BEA4844BF6E6E59A57AF3DD7DC1C62507F0F7E66F8C87EBFE007405C36. Root visually inspected the 560-pixel candidate workspace-details screenshot. Selection/focus/scroll behavior matched the frozen comparison, including the already-documented selection-marker loss on refresh; this is not a new selection-retention claim.

The explicit late-old-writer test passed in 0.79 s. The frozen old process appended 1043 bytes; current verification diagnosed legacy source inventory/hash drift, retained the full appended legacy bytes, and left SQL and the frozen snapshot unchanged. This fixture is intentionally divergent after that test: do not reuse it as a clean migration fixture or repair/reimport it. Full Rust regression was still live when these records were written. Snapshot-pair browser evidence does not establish actual new-CLI browser reconnect, real-host lifecycle, latency, resource, or freshness acceptance; those gates remain separate.

Evidence: target/verification/task6-9360743-native-create.log, task6-9360743-native-export.log, task6-9360743-native-browser/report.json and screenshots, task6-9360743-native-old-writer.log, and legacy-process-rv4TSJ/late-writer-report.json.

### Actual browser restart and Codex CLI at production 9360743

The retained release executable SHA-256 04D85FC68C4AE08200D7D2F139266D27A8297E8E1BE5DBDA3E7CD5928FC755CB passed the actual HTTP/SSE owner-restart browser harness (root 18567, exit 0), using disposable shared-browser-xK7BFF. MCP PID 38260 remained stable while owner 37004 was replaced by owner 36196 with a different instance identity. The URL, seven workspaces, six tasks, expanded details, 140% zoom, scroll position and keyboard focus matched. The selected marker changed from one to zero, consistent with the separately verified frozen frontend behavior; this is not selection retention. The after.png screenshot was visually inspected. Replacement PID 36196 was absent at the subsequent process check; no termination request was issued. This establishes tested reconnect behavior, not formal latency or resource acceptance.

The installed CLI executable path had changed; the obsolete path failed before starting the host harness or creating a fixture. Read-only discovery located bffc5354119c8421/codex.exe, version 0.154.0-alpha.6.2. The current release then passed the ephemeral actual CLI MCP smoke in codex-host-ejsdmN (root 70785, exit 0, no timeout). Exactly three successful calls performed read-map, set-route and read-map, with repository/worktree binding, route goal and revision-one roundtrip verified. Reassessment passed and a deliberately wrong readback failed its negative control. A subsequent process inventory found no running devmap.exe using the retained candidate executable. The isolated configuration did not modify installed plugins or global configuration. This covers real CLI MCP integration; automatic hooks, desktop navigation and complete host lifecycle remain separate gates.

Evidence: target/verification/task6-9360743-owner-restart.log, shared-browser-xK7BFF/report.json and after.png, task6-9360743-codex-host.log, codex-host-ejsdmN/manifest.json, events.jsonl, report.json and report-validated.json. Full Rust regression was still running when this entry was written. Both measured latency gates remain failed.
