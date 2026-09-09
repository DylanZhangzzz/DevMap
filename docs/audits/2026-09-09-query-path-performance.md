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
