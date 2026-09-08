# DevMap SQLite-compatible Implementation Plan

> **For agentic workers:** Use superpowers:subagent-driven-development for independently reviewable tasks, with parent-owned integration and evidence. Follow checkbox steps and review each task before closing it.

**Goal:** Replace local multi-file operational state with repository SQLite and an on-demand shared core while retaining the exact currently installed frontend and devmap/dock/4 semantics.

**Architecture:** Preserve existing domain types, reducer, frontend resources and public tool signatures. Add a transactional repository store behind compatibility APIs, strict read-only legacy snapshots and migration activation, then consolidate runtime ownership. Durable history and provenance remain distinguishable from current projections.

**Tech Stack:** Rust 1.96+, SQLite via a bundled rusqlite build, existing tiny_http/MCP transport, existing HTML/JavaScript assets. Local Windows verification first; portability remains an explicit release gate.

**Spec:** ../specs/2026-09-08-devmap-simplification-guide/02-technical-framework.md and 03-roadmap.md. User subsequently required exact visual/interaction compatibility and authorized implementation until acceptance in a new worktree.

## Global constraints

- Work only in codex/devmap-sqlite-compatible; no merge, push, publish, live plugin replacement or real repository-state migration.
- Source baseline db696768baf366b442adf6097e0c1f4cec3f7e08 plus the verified installed frontend overlay recorded in runtime-baseline.json. Do not change the two frontend assets to simplify backend implementation.
- Keep devmap/dock/4, public route/capture structures, start_commit, CAS conflicts, retry behavior, confidence, provenance, partial inventories, leases and unknown semantics.
- SQLite lives in canonical git-common-dir/devmap/devmap.db, not per-worktree .git pointers. Durable write defaults: WAL, foreign_keys ON, synchronous FULL, finite contention timeout. Unknown schema refuses writes; corruption does not recreate a database.
- Legacy remains authoritative until explicit validated activation. Shadow imports operate on immutable copies; no persistent dual writer. Context Git stays legacy-only.
- Retain current journal retry equivalence (ignores generated sequence and occurred_at); retries return original accepted records. Worktree scope and session identity remain explicit.
- Every accepted command returns only after commit. Binding history and independent watermarks commit together. New confirmed data forbids rollback to an older legacy snapshot.
- Read-only user paths must not silently create a main database, change schema, or repair domain records. SQLite-managed WAL/SHM locking bookkeeping for an existing database is permitted; a live mutable store must never use immutable mode. Query generation and evaluation time are separate.

## Task 0: Fix baseline and record gates

Files: runtime-baseline.json; target/verification/baseline-*; this plan and its SDD ledger.

- [x] Create worktree from main and identify installed binary/source by complete embedded assets and matching binary SHA256.
- [x] Copy only verified assets/dock.html, assets/metro-core.js, src/dock_asset.rs and their two tests from source worktree; leave original untouched.
- [x] Inspect clean-main baseline test completion and rerun overlay UI contracts and JS tests: 286 Rust baseline, 199 JavaScript and 13 overlay UI contracts passed.
- [x] Commit isolated baseline/spec overlay (520683a); capture resource hashes for later byte equality.

## Task 1: Transactional repository store

Files: create src/store/mod.rs, src/store/schema.sql, tests/sqlite_store.rs; modify Cargo.toml/Cargo.lock, src/lib.rs, src/error.rs only.

Produces RepositoryStore with public `open(&SourceWorkspace) -> Result<Self, DevMapError>`, `open_existing(&SourceWorkspace) -> Result<Option<Self>, DevMapError>` (read-only), `path() -> &Path`, `generation() -> Result<u64, DevMapError>`, `integrity_check() -> Result<(), DevMapError>`, `backup_to(&Path) -> Result<(), DevMapError>`; crate-private `connection() -> &rusqlite::Connection` and `transaction<T>(&mut self, FnOnce(&rusqlite::Transaction) -> Result<T, DevMapError>) -> Result<T, DevMapError>`. Transactions are IMMEDIATE; generation is bumped explicitly by changed domain writes inside transaction, not by no-op retry. Store error conversions preserve failure rather than panic.

Schema carries store_meta (version/repository/common-dir/generation/backend state default shadow), journal_sessions, journal_records (session_id/sequence/event_id/record_json/byte_length), route_records (route_id/revision/request_id/input_json/plan_json), presence_records, binding_records, binding_watermarks, migration_sources and worktree_registry. Add constraints and indexes for real identity/CAS/query keys; JSON holds existing validated payloads. Domain validation remains above this layer; do not expose arbitrary SQL over CLI or MCP.

- [x] Write failing real-store tests for missing read-only open, shared linked-worktree database, WAL/FULL, restart persistence, foreign repository identity, unknown schema and backup rejection of an existing destination.
- [x] Run `cargo test --test sqlite_store` and save the RED evidence.
- [x] Implement minimal store, schema creation under transaction, version/identity checks before changing existing DB, safe file/sidecar checks using existing fs_security, finite busy timeout, consistent backup to new destination.
- [x] Add transaction rollback/contention/integrity and active-WAL backup restore checks; run `cargo test --test sqlite_store` plus store unit tests. Example behavior: failed transaction must leave generation unchanged; a second connection sees only committed rows.
- [x] Parent reviews public interface and tests; commit task-only paths after fixes.

## Task 2: Route and task binding backends

Files: src/route_plan.rs, src/journal.rs binding portion, new src/store/routes.rs and bindings.rs as needed; tests/sqlite_routes.rs and sqlite_bindings.rs.

Consumes RepositoryStore. Keeps RoutePlanStore::open/list/list_with_starts/set and read_task_bindings/observe_task_bindings callers unchanged. Use validated active-backend lookup: no active SQL means existing legacy behavior. SQL write opens its own validated writable connection; reads use open_existing. Add explicit read-only legacy snapshot helpers that expose full route input/revisions and binding watermarks without creating locks or recovery writes.

- [x] First tests activate only disposable stores and assert `first.start_commit == updated.start_commit`, identical request returns original result, changed request conflicts, competing expected_revision has one winner.
- [x] Run focused RED checks, then factor domain validation/building from file IO without weakening existing tests.
- [x] Implement route records and starts plus binding-history/watermark updates in single transactions; preserve unchanged-association watermark advancement, late observation rejection and partial-list rules.
- [x] Compare serialized legacy and SQL outputs from the same imported records and fixed evaluation time; cover removed-worktree abandonment, missing target validation, pending legacy watermark rejection.
- [x] Run original route_plan and binding-focused dock_model tests plus new tests; review then commit.

## Task 3: Journal and presence backends

Files: src/journal.rs, src/presence.rs, src/capture.rs/hook.rs/mcp.rs at acceptance seams; tests/sqlite_journal.rs, sqlite_presence.rs.

Keeps JournalStore::open/append/append_batch_with/replay/session_id, CaptureKernel constructors, PresenceStore APIs and JournalSummary. Store wrappers retain workspace/session identity instead of creating legacy directories when SQL active. Accept each journal batch once; preserve event hash chain and sequence. SQL summaries do not repair state. Imported explicit presence is durable enough to preserve waiting states not derivable from event history.

- [x] Write RED tests for generated sequence, hash chain, retry with changed occurred_at, changed payload rejection, partially repeated batch rejection and restart summary.
- [x] Implement SQL append/replay with existing validation/hash helpers; read session registrations across common/linked worktrees, detect duplicate session origins rather than choose one.
- [x] Add acceptance/projection transaction seam so duplicate records do not increment gap counts or renew leases. Preserve explicit host signals and record their accepted projection state. Document this specific duplicate-projection bug correction separately from equivalence checks.
- [x] Test crash/rollback boundaries, tampered records, session mismatch, retired worktree identity, missing/corrupt summaries and leases at fixed time.
- [x] Run journal_flow, final_review_journal, presence_store, final_review_capture and new focused suites; review then commit.

## Task 4: Frozen snapshot import, verification, activation and recovery

Files: src/store/migration.rs, CLI storage subcommands and dispatcher, tests/sqlite_migration.rs; compatibility fixtures/reports.

Produces explicit migration APIs and CLI surfaced through `devmap storage` (inspect, migrate, verify, backup; final arguments fixed in task before implementation). Migration reads strict frozen legacy snapshots, tracks source hashes/provenance and record counts, writes a shadow store, compares all input contracts, then activates only after evidence validates. Direct JournalStore::replay is forbidden for snapshot import because it mutates files. Runtime auto-setup may create a fresh SQL store for a genuinely empty repository; existing legacy data requires safe startup freeze/backup verification before activation.

- [x] Tests first: mixed common-dir/per-worktree sessions, newer unchanged binding watermark, original IDs/revisions, repeat import, truncated journal, pending intent/watermark, unsupported objects, source drift, partial import failure.
- [x] Import transactionally from validated snapshots; Context objects remain accessible in original legacy source, explicitly recorded legacy-only. Do not delete source files.
- [x] Verify old/new reducer inputs and DockReadModel under fixed time; preserve exact schema, sorting and warnings. Build a durable activation record and reject old schema/new data downgrades.
- [x] Test backup while WAL active, restore, activation interruption and retry; do not replace open SQLite handles on Windows. Refuse rollback after new writes unless lossless reverse export has been verified.
- [x] Run migration + existing read-only and map suites; review then commit. No production repository migration during tests.

## Task 5: Shared runtime and user-invisible startup

Files: src/runtime/ module, CLI hidden runtime entry, mcp.rs, viewer.rs, hook.rs; tests/shared_runtime.rs and process fixtures.

The shared owner is keyed by canonical common dir. IPC has bounded typed requests and response framing, repository/protocol handshake, serialized command acceptance, multiple reader clients and finite waits. Use same-user local IPC (Windows named pipe; Unix socket) with no arbitrary command execution. Keep existing MCP entrypoints and frontend URLs/resource schema compatible; browser remains read-only. Separate proxy/session from shared heavy state.

- [x] Write process tests first for simultaneous start (one owner), four clients, disconnection independence, wrong-repository/version rejection, owner termination/reconnect and same-id retry after lost response.
- [x] Implement startup election, liveness validation beyond PID alone, immutable handshake, bounded queue and idle exit; never kill unknown PID or silently create a second writer.
- [ ] Route existing commands through common application services; avoid per-client duplicated Git scans/watchers. Preserve no-listener-on-MCP-before-explicit-browser contract.
- [ ] Add on-connect catch-up, change hints and bounded Git ref reconciliation; report stale/partial/error per source. Snapshot reads pin generation and evaluated_at; paging detects generation drift.
- [ ] Exercise startup migration and old-writer detection on disposable repos; failed pre-activation migration continues legacy without losing records, while post-activation failures preserve SQL and show diagnostics.
- [ ] Run process/runtime, hook/MCP/viewer and complete Rust suites; review then commit.

## Task 6: Whole-branch compatibility and acceptance

Files: tests/browser/sqlite-compatibility.cjs or target/verification scripts, docs/audits/2026-09-08-sqlite-compatibility.md, docs/installation.md and storage operations guidance.

- [ ] Assert verified installed asset SHA256s unchanged; run JS renderer/metro core and Rust UI contract tests.
- [ ] Capture old/new real browser render with identical frozen inputs at desktop/sidebar widths; compare screenshots and interaction results (selection, zoom/pan, expansion, refresh/reconnect) including legitimate dynamic exclusions explicitly documented.
- [ ] Build release binary and exercise local real MCP stdio + browser + host event loop using isolated repo and configuration; distinguish host integration fixtures from real host evidence and leave unproven host claims open.
- [ ] Measure cold open, warm query p50/p95, small Git-change freshness, CPU/RSS, concurrent owner count and summary bytes against roadmap budgets; report hardware, sample counts and failures. Fix failures or retain failed gates; do not lower guarantees to claim success.
- [ ] Run `cargo test --all-targets --no-fail-fast`, `cargo fmt --check`, relevant clippy/packaging checks and independent whole-branch review. Audit every P0–P5 requirement against current evidence before goal completion.
- [ ] Keep branch/worktree for user review; no merge, push, install or publication without a subsequent explicit request.

## Rulings and progress

The plan-scoped SDD ledger is authoritative for per-task progress, review findings and decisions. Completion of this plan requires the entire objective, not just an unused SQLite prototype. P1 being green cannot close the goal.
