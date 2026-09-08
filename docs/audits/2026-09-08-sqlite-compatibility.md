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

Application commit `56c927a` is not approved yet. Independent source review found
that partial inventory merges can exceed the accepted query limits after binding
writes, and that shared relationship observations can incorrectly reuse the
owner worktree's `devmap.developmentTarget` for another worktree. Both require
focused regressions and fixes. The separately reviewed `d9c71bb` wire seam was
approved; this does not approve the unfinished application integration.

Transport review fixes remove detached identity workers and correct the rejected
Hello test oracle. The current working patch still needs full ancestor permission
validation and owner-lock retention through reactor teardown. Ten process tests
passed before the final startup guard; later unit/clippy checks passed, but the
final combined link failed because C: was full. Do not count that failed run as
validation of the final patch.

Builds stopped when C: had no space. Generated files, fixtures and uncommitted
changes were preserved. Space later recovered to about 1 GB on C: and 3 GB on D:;
subsequent verification uses a separate D: target with incremental compilation
and debug symbols disabled. This changes the local verification environment,
not the durability, compatibility or final performance acceptance criteria.
