# Query path milestone: compatibility passed, performance still open

The read-only shared executor retains a physically sealed source and a bounded, rechecked plain Git configuration proof. Failed connection-time physical capture is permanently refused for queries. Mutations and inventory acceptance retain their independent fresh validation. The frontend assets, wire schemas, Git observation timestamps and production two-second Git cache age are unchanged.

Nine query regressions passed, including missing global configuration creation, byte-identical configuration replacement, worktreeConfig fallback, HEAD movement/expiry, moved/recreated sources and failed connection seal repair. Four process/application integration targets passed 31 tests. Renderer/core/packaging JavaScript tests passed 205 tests. Bounded independent source review found no remaining blocker. These checks do not constitute the final whole-branch or performance acceptance.

## Real Release smoke

Candidate executable SHA-256: `ed78a4dba5e928066193312899f7afbbc4058467f8c576c5b48856436b6d5bc4`. Built from `988663b` plus this query-validation milestone. All runs used explicitly owned fixtures; installed runtimes and user repositories were untouched.

| Scope | Observed result | Gate |
| --- | --- | --- |
| Tiny owner-cold, two observations | 3910 / 3841 ms | 3000 ms, failed |
| Tiny four-client hot subset, eight observations | approximately 197–812 ms | 200 ms per-client p95, failed |
| Tiny freshness, four measured changes / 16 detections | max-of-four p95 3787 ms | 2000 ms, failed |
| Complete summary result | maximum 5431 bytes | 32768 bytes, passed in this smoke |

The tiny fixture contains two real worktrees, two sessions and six journal records. Runs `shared-summary-S24xdI` and `shared-freshness-Qn5ja8` completed response/cursor, full SQL/backup preservation and owned cleanup checks. They are not substitutes for the fixed scale populations.

Earlier failed run `shared-summary-t48DR0` remains retained. Its second query spanned approximately 70 minutes of Modern Standby, verified against Windows System Kernel-Power events 506/507. The first, pre-standby cold observation was 6125 ms. No failure was deleted from its report.

## Optimized in-process diagnostic

The exact-receipt ignored profiler uses a controlled 60-second hot window solely to isolate costs. It does not measure production TTL, transport latency or concurrent clients. Stages are independent calls, not additive spans of one request. The scale fixture contains 20 real worktrees, 100 sessions and 100000 records; its public host-observed task count is zero, distinct from stored task entries.

Run `query-stage-LkE6Ca`, optimized test executable SHA-256 `acdc18a7c39fc047be64d71782c5ee1cb825bab52fd7dd2eb75d74cd20143fc7`, completed five samples per repeated stage and preserved all SQL tables and backups. The owned Windows Job ended empty with root exit code zero.

| Stage | Mean milliseconds | Git starts per call |
| --- | ---: | ---: |
| Connection physical validation | 1.29 | 0 |
| Source physical validation | 1.17 | 0 |
| Configuration witness recheck | 3.79 | 0 |
| SQLite existing-store open | 1.48 | 0 |
| Activation/provenance validation | 7.07 | 0 |
| One current-origin enumeration | 162.30 | 1 |
| Full legacy inventory and hashes | 342.96 | 0 |
| Full active-origin observation | 635.59 | 2 |
| Complete sealed hot query | 710.08 | 2 |

Initial sealed query took 8096 ms and 66 Git starts. Independent cold storage read took 2453 ms; later reads still required origin and legacy-byte validation. The first diagnostic deliberately exposed the mistaken zero-Git assumption on active SQL and failed; the diagnostic now records actual counts while retaining model/cycle/time assertions. The native plain-fixture zero-Git regression remains narrower evidence.

The remaining work is to reduce repeated origin enumeration and full legacy scan cost while retaining moved/replaced/unavailable-origin behavior, malformed-input errors and byte-integrity checks. Do not infer that removing two Git starts alone meets the gate. Final scale cold/hot/freshness/resource populations, final candidate browser/host checks, the complete Rust suite and whole-branch review remain open.

## Bounded streaming inventory follow-up

Inventory hashing now reads through a 64 KiB buffer and checks actual bytes against the remaining aggregate budget. The prior metadata-only precheck could admit a file that subsequently grew beyond that budget. Two real-file boundary/growth tests first failed against the extracted previous behavior; all five Windows controls now pass, including hash equivalence, hard-link rejection, interrupted/short reads and error propagation. Other migration reads retain their existing implementation. All 36 migration/origin/move integration tests passed and bounded source review approved this slice.

Optimized scale run `query-stage-R7rhTH` (test SHA-256 `f49b1fe65a5123fdfad88675920c5faf2910e6db4f7fd75d27f4a4b4ab23c431`) preserved SQL and backups and ended with an empty owned Job. It does not establish a latency improvement: complete inventory averaged 364 ms and sealed hot query 719 ms. Independent file-only diagnostics measured approximately 206 ms for checked opens, 17 ms for reads and 45 ms for SHA updates/finalization. These measurements identify filesystem work as a substantial cost; they do not replace the full production timing population. The bounded buffer is an implementation bound, not a measured whole-process RSS result.

## Discovery dependency correction

A later real native-fixture regression exposed a missing dependency in the first query proof: moving the owned `objects` directory made fresh Git discovery fail, while a sealed hot query still returned its cached cycle. That RED is retained in `task6-query-objects-red.log`; the earlier milestone's narrower tests did not cover this case. Git's discovery scaffold is documented in [Git 2.45.1 setup.c](https://github.com/git/git/blob/v2.45.1/setup.c#L322-L369), and the local executable reports `2.45.1.windows.1`.

The query proof now includes accessible objects/refs directories and their physical identities, common/admin HEAD and packed refs, and a bounded complete loose-ref tree. Every recheck rebuilds the filesystem evidence, so directory additions and byte-identical physical replacements invalidate it. A fresh authoritative inspector runs after candidate capture and before its final recheck; unsupported or changed evidence retains the original fresh path. Config plus source-file evidence retains the combined 4 MiB byte bound. This is not an object-content integrity scanner or an alternative Git ref parser.

All 16 query regressions passed, including refs/HEAD removal, malformed HEAD/current ref, and objects/HEAD replacement. A valid commit invalidates the source proof and triggers fresh validation; an immediately subsequent unchanged call again starts no Git process in the native fixture. Original model/cycle/time assertions and real TTL expiry remain intact. These counts must not be generalized to active SQLite, whose separate origin observation still performs Git enumeration. The previous Release performance numbers predate this correction and are not acceptance for the final candidate.

The correction also passed all 31 process/application integration tests (origin reanchor 3, MCP 11, repository application 10, shared application 7), recorded in task6-query-discovery-integration.log. The serial runner exited zero. cargo fmt --check and cargo clippy --all-targets -- -D warnings passed. Independent bounded source review found no remaining blocker for this ordinary-layout proof; final whole-branch and performance acceptance remain open.

## Current discovery-corrected Release smoke

Release a8baef7, SHA-256 65efc63acbc6a6306c0e2db8aeb5baf5471f4d8a4706c6aea3dd30d62d8249de, built successfully in task6-discovery-release.log. Tiny owned run shared-summary-HaTwRQ completed all request, cursor, SQL/backup preservation and cleanup checks with no errors (runner exit zero). Two cold observations were 4408/4560 ms; eight hot observations were approximately 230-934 ms, with per-client p95 254/934/769/703 ms. Complete result maximum was 5431 bytes. Cold and hot gates remain false. This is a smoke baseline for the corrected proof, not the fixed scale population or evidence of improvement. Frontend asset hashes remained identical.

The next origin-enumeration slice has a retained actual RED: both main and linked active SQLite repeat reads start two Git processes, where the proposed reuse test requires zero. Full legacy-byte drift rejection already passes. The first fixture attempt lacked presence records and failed before measuring Git; that failed setup log is retained separately and is not the performance RED. No performance gain is established by adding these tests.

## Resumed origin-enumeration cache validation (2026-09-11)

The retained implementation initially did not compile because its child module used the wrong relative path. After fixing that path, both main and linked repeat-read tests still failed with seven Git starts: diagnostic evidence showed the physically identical main workspace used a Windows extended canonical path in the proof and Git's ordinary path spelling in the scan. The proof now compares checked canonical root paths while returning the original Git-scanned origins unchanged.

Five focused tests passed (task6-origin-cache-first-differential.log): active SQL main/linked repeat reads start zero Git processes, legacy-byte drift still fails, a new worktree invalidates and matches a fresh reader, and non-UTF8 locked metadata retains the fresh scanner error then reacquires after repair. Six integration tests passed in task6-origin-cache-move-integration.log, covering application reanchor, same/foreign old-path occupants, native/frozen moves and frozen-byte tamper. Formatting and warnings-denied all-target Clippy passed after collapsing one nested conditional.

This slice remains uncommitted and has no current Release performance measurement or independent approval. The broader invalidation matrix, explicit aggregate path-evidence bound, between-guard drift control, full branch suite and final scale/browser/host acceptance remain open. Zero Git in an isolated storage repeat read is not a latency or whole-query acceptance claim.

## Origin-cache boundary validation and measurement

The explicit 1 MiB path-evidence budget first failed its exhausted-budget test, then passed with checked admission before recording directory or missing-file evidence. Nine focused tests now cover a real worktree addition between guards with refusal, cache discard and successful reacquisition; direct original uncached observer comparison after another linked worktree's HEAD changes; and main/linked/main switching with complete presence, journal, route and binding input equality. Independent source review approved this bounded slice. Seventeen application/shared-runtime integration tests also passed before the repeated-proof checks were consolidated.

The first optimized scale run, query-stage-XmJGXc (test executable SHA-256 d03a29a60320e5bb3b7a2cbb12b1bdebe55876fb900495885528b3a5b19d3264), preserved SQL/backup state and finished with an empty owned Job. It measured zero Git starts but a 778 ms mean sealed hot query, with warm storage reads about 673-709 ms. This did not demonstrate a latency improvement. Four full filesystem-proof checks per hot storage read were then consolidated into the entry and final-return checks of the same immutable proof. Original full inventory hashing and unavailable-path checks remain. Provider clones are not independent live scans. Neither version is an atomic filesystem snapshot; fewer samples can miss some transient changes that are restored before final validation, so this is not a claim of identical rejection at every possible instant. The nine focused controls passed again (47.85 seconds), including persistent between-guard change rejection, and formatting/all-target Clippy passed. Final multi-client and branch acceptance remain open.

The two-boundary optimized run query-stage-jq1B94 (test executable SHA-256 80eba4c629f1991482713e19c6e414ae1210150d2683981bc1eed4bfae7be570) completed with no errors, an empty owned Job and full SQL/backup preservation. Five hot observations averaged 557 ms with zero Git starts, versus 778 ms in the preceding four-check diagnostic; warm storage reads were 464-485 ms. Independent full legacy inventory remained approximately 278 ms, and the original uncached full observer averaged 566 ms. The single initial query measured 9166 ms / 73 Git starts. This is a roughly 28 percent reduction in the diagnostic hot mean, not the required production four-client p95 population or cold acceptance. Source proof and configuration still require filesystem work; removing subprocesses did not remove that cost.

## Full-suite result and retained scale failure

The origin-cache slice was committed as 31784e8. Its complete serial `cargo test --all-targets --no-fail-fast -- --test-threads=1` run finished with exit code 101: 552 passed, one failed and eight ignored. The sole failure is `budget_preserves_all_256_real_workspaces_and_late_dirty_unprotected_facts`, which returned `GitProcess(Deadline)` at tests/dock_model.rs:1310 before model-budget assertions. All SQLite migration, identity, move, concurrency and startup-activation targets passed. The full log is retained as task6-all-targets-31784e8.log; this is not a passing branch result.

An isolated rerun of the same compiled test reproduced the same deadline failure in 68.35 seconds, including fixture construction. Its eligible Git Trace2 sink is retained in task6-256-trace-31784e8.jsonl alongside task6-256-isolated-31784e8.log. Excluding worktree setup, the final refresh begins at 15:33:28.360 UTC; 256 status commands start between 15:33:32.303 and 15:33:45.645. Shared-proof acquisition proceeds afterward, and a merge-base starts at 15:33:53.237, near the original 25-second deadline. Trace records cannot alone attribute intervals outside Git to admission, process startup, cleanup or filesystem work. No operation budget, fixture population or acceptance threshold has been relaxed. Phase attribution and a verified fix remain open, along with final performance and browser/host acceptance.

An opt-in, test-only stage profiler then completed ten sequential real `git --version` calls with valid output and exit status. Mean total time was 96.95 ms: suspended process creation 5.33 ms, Job attachment/thread discovery/resume 38.13 ms, root wait with concurrent pipe reads 39.76 ms, and cleanup 13.41 ms. Reactor creation/destruction together averaged 0.24 ms. These are Windows debug diagnostic measurements, not scale latency acceptance or attribution of every millisecond inside attachment. The inclusive total and outer measurements must not be added to their component spans. Error-path span labels do not establish successful cleanup, and the captured profile covers the explicit reactor thread, not arbitrary application workers.

The profiler and eight existing process fault regressions passed; formatting, warnings-denied all-target Clippy and independent bounded source review passed. The diagnostic adds no production profiling behavior. Its raw samples and statistics are retained in task6-git-process-stages-first.log and task6-git-process-stages-first-statistics.json. The production fix remains pending.

## Target-process thread discovery candidate

The Windows candidate retains suspended creation and assignment to a kill-on-close Job before execution. It replaces global thread enumeration with [PssCaptureSnapshot](https://learn.microsoft.com/en-us/windows/win32/api/processsnapshot/nf-processsnapshot-psscapturesnapshot) of the held child handle, capturing only thread records. Bounded enumeration requires one live candidate; the opened thread must match the process and creation time before resume. Capture unavailability retains the original Toolhelp path; later identity, resource, release or resume failures do not retry another path. This is not an older-Windows loader fallback claim. [PssFreeSnapshot](https://learn.microsoft.com/en-us/windows/win32/api/processsnapshot/nf-processsnapshot-pssfreesnapshot) uses the current process for the locally captured snapshot.

The retained test-first run had one passing fallback control and four expected failures. The first implemented run passed all five owned-child controls; eight original process regressions also passed. In the same ten-command diagnostic, mean attachment fell from 38.13 ms to 0.165 ms, and total command time from 96.95 ms to 62.24 ms. Cleanup averaged 20.70 ms versus 13.41 ms previously, so no cleanup improvement is claimed. Raw logs are task6-pss-first-green.log, task6-pss-process-regressions.log and task6-git-process-stages-pss.log.

The unchanged 256-worktree case then passed in 47.31 seconds including fixture construction, with the original 25-second refresh budget and all model assertions intact (task6-256-pss-first.log). This is one isolated regression pass, not a new passing whole-suite or production performance population. Independent review found no source blocker and requested additional opened-handle identity controls before the milestone is finalized. Final whole-branch, four-client and UI acceptance remain open.

The additional controls now pass: opened-thread owner/creation mismatch and controlled marker/snapshot release refusal. All nine PSS tests and the owner-quarantine regression passed, as did formatting, warnings-denied all-target Clippy and the final bounded source review. Failure controls use actual owned child processes and assert exact errors and no fixture execution after reaping. Release and resume failures are controlled branches, not observed Windows API failures; these tests do not independently count successful snapshot frees on all error paths. Logs: task6-pss-identity-controls.log, task6-pss-quarantine.log and task6-pss-clippy.log. Whole-suite and production performance acceptance still require the final candidate.

The slice is committed as ad18d83. Its Release executable, SHA-256 654816ddfc3ecff45dd0609f934733d27bb77203636da6fc956456273dc68a58, built successfully and completed tiny real MCP run shared-summary-Q2KPRX with no request or cleanup errors and full SQL/backup and cursor preservation. Two cold observations were 3477/3368 ms. Twelve hot observations had per-client p95 values 459/203/445/461 ms; maximum complete result remained 5431 bytes. Cold and hot gates remain false. This small smoke does not replace the fixed scale population, final freshness cohort, idle resources or UI/host verification. Logs and configuration are task6-pss-release.log, task6-summary-pss-smoke.log and task6-summary-pss-smoke-config.json.

## Cold inspection consolidation

A native main-fixture RED recorded 30 total cold Git starts and two authoritative inspector groups of four commands each. The cold query now captures candidate configuration from authenticated paths, performs one authoritative inspector, compares canonical identities, and only then constructs the application. Proof checks before and after projection, failed-Hello seals and the original unsupported/changed-evidence fallback remain intact. Reordering can change which simultaneous race error is encountered first; this is not identical error-priority timing under every filesystem interleaving.

All 19 query-validation tests passed. Recorded cold starts were main 26, linked 31 and unborn 24; their single inspector groups used 4, 4 and 6 commands respectively. The main observation is a four-command reduction from its actual RED; the other layouts verify one inspector, not a measured before/after total. Warm zero-Git and original cycle/time assertions passed. Objects removed after candidate capture prevent application creation; configuration changed after inspection produces the fresh alternate target and does not retain the stale proof. Formatting, warnings-denied all-target Clippy and independent source review passed.

The retained logs are task6-cold-inspector-red.log and task6-cold-inspector-query-matrix-after-module-fix.log. An initial combined build failed on the separate inventory prototype's module path before running tests; task6-cold-inspector-query-matrix.log records that compiler failure. After correction, the separate parallel-inventory prototype still has its intentional RED (three failed tests, one passing serial-fallback control); that work is not included in this cold-path milestone. The latest Release measurements above precede this consolidation, and all final acceptance gates remain open.

## Bounded parallel inventory milestone

The candidate enumerates and classifies metadata through the original walker, then uses at most four scoped payload readers to compute every full digest. Each reader keeps a 64 KiB buffer. Complete unique ordinals, exact observed byte lengths and aggregate admission limits are required before returning a manifest. Every worker is joined before a decision. Ordinary speculative failures fall back once to the original serial inventory; worker panic returns a controlled error after joining. Stable successful manifests match the serial oracle. Transient failures can be repaired before fallback, so identical error timing across every interleaving is not claimed. The 512 MiB admission limit applies per attempt; fallback can perform another attempt, and failed readers can consume one additional EOF-probe byte each. Safety checks may open additional temporary handles.

Fourteen streaming/parallel controls and all 36 migration, native/frozen origin lifecycle and worktree-move integration cases passed. Controls include actual growth/shrink, hard links, a Windows junction inserted after enumeration, complete manifest equality, quota rejection, duplicate results and panic cleanup. The first duplicate-result fixture accidentally triggered length rejection first; the corrected fixture duplicates the same successful record. The junction fixture initially passed forward-slash paths to mklink and failed before its intended assertion; only its owned fixture path spelling was corrected. All failure logs are retained. Formatting, warnings-denied all-target Clippy and independent bounded source review passed. Actual OS worker-spawn failure and Unix execution were not established by these controls.

Optimized diagnostic query-stage-JnCWRU used test executable SHA-256 763fbc0ba271b907039d1427df24712386b748d44aa14a9f0b74548198038b75 on the fixed 20-worktree, 100-session, 100000-event receipt. Five paired inventories alternated order and compared their entire FrozenManifest against each other and the frozen baseline. Serial inventory averaged 311.820 ms and parallel inventory 150.795 ms, a 51.6 percent reduction for this paired stage. Full sealed hot queries averaged 512.648 ms (493.848-535.418 ms), each with zero Git starts. Application verified aggregates averaged 428.220 ms. The single initial query was 6669.679 ms / 69 Git starts. These are diagnostic observations with the controlled profiling TTL, not the required production four-client p95 population or cold acceptance. The hot target remains unmet.

The owned runner exited zero with an empty Job, no errors, all 14 SQL tables and complete backup inventories preserved. Logs are task6-inventory-parallel-streaming-controls-fixed-path.log, task6-inventory-parallel-migration-regressions.log, task6-inventory-parallel-clippy.log, task6-inventory-parallel-release-profiler-build.log and task6-inventory-parallel-scale-profile.log; statistics are task6-inventory-parallel-scale-statistics.json. This executable is the optimized library test harness, not a rebuilt CLI. Final CLI, complete branch suite, production performance, browser and host acceptance remain open.
