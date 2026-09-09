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
