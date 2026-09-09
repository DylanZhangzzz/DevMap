# Explicit schema-2 fixture generator

Root execution update (2026-09-09): owned Job normal completion and forced abort with a live descendant both confirmed emptiness. Wrong receipt seal, wrong physical identity and a non-pristine allocation were rejected by the real compiled Rust entry without changing their allocation contents. A new C: tiny allocation completed migration, full public summary/detail finalization and preservation. Full-scale generation is separately tracked; tiny success is not scale or performance acceptance.

On this host D: is suitable for retained harness artifacts but its root ACL prevents repository storage use: journal transition validation checks the entire Git common-directory ancestor chain. A private D: leaf plus trusted C: TEMP does not remove that restriction. Root used the existing trusted C: test parent for corpus generation and did not change D: ancestor ACLs.

Implementation only; do not execute generation until root reviews the new owned parent, disk and memory headroom, candidate/test binaries and configuration. No corpus or allocation was created while implementing this code. The old `generate_disposable_legacy_scale_corpus` body is unchanged.

The new ignored Rust entry is `generate_owned_schema2_scale_corpus` in `tests/performance_fixture.rs`. It is not exercised by normal tests. Root must compile the test executable separately; the Node orchestrator never invokes Cargo.

Configuration requires:

```json
{
  "mode": "smoke",
  "parent": "D:/explicitly-approved-private-parent",
  "approved_private_parent": true,
  "candidate": "D:/root-reviewed/devmap.exe",
  "candidate_sha256": "actual 64-character hash",
  "test_executable": "D:/root-compiled/performance_fixture-test.exe",
  "test_executable_sha256": "actual 64-character hash",
  "python": "C:/explicit/runtime/python.exe",
  "trusted_temp": "C:/Users/user/.devmap-test-fixtures/devmap-sqlite-compatible-20260909",
  "evaluation_time": "2026-09-09T12:00:00Z",
  "dimensions": {"worktrees": 2, "sessions": 2, "events_per_session": 3}
}
```

The timestamp is an explicit fixed synthetic evaluation time, not a claim of real host observation. `scale` always means 20 real worktrees, 100 sessions and 1000 events/session; it does not honor silent dimension reductions. Smoke permits 1–4 roots, 1–8 evenly distributed sessions and 1–20 events/session and records actual dimensions. Neither generation nor receipt publication is performance acceptance.

```powershell
# Read-only; requires real config paths/hashes, but creates no allocation.
node tests/browser/generate-schema2-fixture.cjs --preflight C:/reviewed/config.json

# Mutating generation; ONLY after separate root authorization.
node tests/browser/generate-schema2-fixture.cjs --generate C:/reviewed/config.json
```

Preflight checks absolute paths, existing ancestor symlink/reparse shapes through the shared checked-path helper, explicit executable hashes and available disk. It reserves 512 MiB, 128 MiB future harness logs, the candidate size, and eight times the planned ≤1024-byte normal-record payload plus one Unicode outlier. It does not certify ACL trust or memory capacity: `approved_private_parent` records the operator's earlier review. The script never changes a parent/root/system ACL; unsafe runtime storage permissions remain an error. Root must separately inspect memory before executing scale.

`trusted_temp` is mandatory and must be an existing root-reviewed runtime-safe directory. Its canonical identity is rechecked and passed as child-only TEMP/TMP. The example is the C: test directory root has already reviewed; no directory or ACL is created/changed by this setting. D: corpus allocation is not advertised as a safe runtime IPC ancestor chain. Production runtime permission checks remain in force.

Node exclusively creates two random nonce directories: fixture allocation and generation-report allocation. Creation evidence is written immediately to the separate report allocation. A Python standard-library `GetFileInformationByHandle` probe records volume serial/file index using the same native algorithm as the existing Windows directory-identity tests; Node dev/ino are retained separately, not assumed equivalent. The allocation receipt is written once. Its SHA-256 is passed to the exact ignored Rust entry through child-only environment. This seal is an integrity token, not a security authority; exclusive creation, retained physical identities and explicit parent approval establish scope.

Rust refuses a directory containing anything except that initial receipt, revalidates its physical identity/seal at stages, and never resumes/adopts partial work. It commits a tracked small probe, creates actual Git worktrees, then generates canonical synthetic journal batches and presence through the existing legacy APIs. The first actual session remains part of the final corpus and supplies the payload pilot; normal serialized records over 1024 bytes abort with retained evidence. One final event in the first session uses a bounded synthetic Unicode actor. The finalizer must verify a real multi-chunk Unicode public detail; source string length alone is insufficient.

After proving no DB exists, Rust calls public `freeze`, `import_shadow`, matched-snapshot comparison, `activate`, and `verify`. No SQL row is manually inserted or selector manually changed. SQLite integrity/FK checks and schema version are read afterward. `manifest.json` includes actual root/admin physical identities, worktree IDs, sessions/final hashes, generation, fixed evaluation time, payload size and immutable frozen path. Append-only Rust stages retain progress; failure never deletes files or tries recovery.

Node then starts only its retained explicit owner and MCP proxy, refuses an occupied endpoint, verifies Hello PID/build/nonce and fixed source/common identity, and reads the public summary without host inventory. It enumerates all three collection chains and all details with stable envelope, byte/offset/hash checks, unique cursor tokens and the Unicode requirement. Actual summary counts are recorded rather than inferred from 100 sessions. Shared exported SQL/inventory helpers compute the exact benchmark digest format before and after public reads. All 14 table contents and the frozen inventory must agree.

Only after retained children close successfully can `benchmark-receipt.json` be created. It follows `devmap/benchmark-fixture/1`, records actual dimensions and physical directory IDs, and is intended for the separate latency harness. No candidate installation, real user migration, recursive delete or PID-discovered kill is performed. Generation has a 30-minute child bound and 64 MiB log bound; failed or interrupted allocations remain deliberately unpublishable and require explicit operator inspection.

The mutating Rust generator is now launched through `windows-owned-generator-job.py`: CreateProcessW creates it suspended, assignment to a private kill-on-close Job precedes ResumeThread, and breakaway is not permitted. Normal root completion and an abort through the retained wrapper's stdin both require Job ActiveProcesses=0, recorded in an exclusive `rust-job.json`. A surviving descendant after the bounded grace period is terminated through the owned Job handle and makes normal generation fail. Node waits for the wrapper and checks this receipt; an unresponsive wrapper can be closed to trigger kill-on-close, but absent emptiness confirmation is still a cleanup failure, never a success claim. The wrapper's SHA-256 is retained. No PID enumeration or broad kill is used. This dedicated wrapper only encloses the test generator and its Git descendants; it does not replace the production owner's existing process supervision.

Before launching the public-summary owner, the finalizer now checks containment of every manifest source/root/admin/common/database/backup/probe path and verifies recorded root/admin physical identities. It also verifies actual SQL activation snapshots lie in the allocation and are covered by immutable inventories. Candidate hash is rechecked before the read-only runtime-identity invocation as well as owned launches.

Verified by this worker: Node syntax, Python AST syntax without launching the wrapper, and pure dimension-validation assertions only. Root subsequently reported all-target Clippy compilation of the Rust generator succeeded. Actual smoke must cover pristine-allocation refusal, mismatched seal/identity, successful Job completion and forced-abort descendant cleanup, failure retention, migration and public receipt finalization before scale is authorized. Source review and parsing do not prove Windows Job behavior on this host.
