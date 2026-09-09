# Task 6 — Native origin identity, schema 2 implementation plan

Status: DESIGN ONLY. Root accepted the direction; implementation remains separately authorized after schema-1 concurrency acceptance and scoped commit. This document adds no production/test changes and no Cargo execution. Preserve all existing schema-1 performance corpus, negative logs, comparison snapshots and backups.

## Problem and invariants

Activation manifest origins describe the frozen baseline, not every worktree created later. Journal writes register native incarnations, but route_plan::set and journal::observe_task_bindings currently do not. Therefore a post-activation worktree can be removed/recreated and its old route and binding can attach by path-derived worktree_id. Registry-wide suppression alone is also wrong: after recreation, a correctly created new route must attach while the old route remains historical.

The SQL schema version becomes 2 explicitly. Public RoutePlan, TaskBindingObservation, mutation requests/results and Dock JSON keep their existing schemas and fields. Preserve old domain bytes and provenance; no automatic retired_at, synthetic movement claim, cross-incarnation session reuse or source identity repair.

## Minimal internal data model

Retain existing domain tables and JSON. Add three internal tables, all STRICT with checked qualification tags, primary keys, referential constraints and bounded field validation:

1. route_origin_links: (route_id, revision) primary key/FK to route_records; worktree_id, nullable incarnation, qualification. Known links reference worktree_registry(worktree_id, incarnation). Link worktree_id must equal the corresponding plan/input target. Qualifications native_verified, frozen_baseline, unknown. Known means non-null incarnation; unknown requires NULL. Exactly one link per route revision; missing link is corruption, not an implicit unknown.

2. binding_origin_links: observation_id primary key/FK to binding_records; destination_worktree_id, destination_incarnation, destination_qualification; source_worktree_id, source_incarnation, source_qualification. Destination ID must match record.worktree_id. Source ID must match record.from_worktree_id; distinguish absent source (not applicable) from a present but unknown source. Known source/destination pairs reference registry. Require exactly one link per binding record. Link identities are immutable with their record.

3. binding_origin_cursors: source_scope primary key/FK to binding_watermarks; observed_at, current_worktree_id, nullable current_incarnation, qualification, nullable history_observation_id FK to binding_records. observed_at must equal the canonical watermark time. This is the newest accepted inventory association, not a historical migration record. Validate history record's host/task against source_scope when present. A cursor may become newer than its history record after unchanged or same-path/new-incarnation observations; never rewrite that record's identity link.

The cursor is essential: update_bindings currently skips same worktree_id, while parse_binding_records explicitly rejects same-ID migration records. Do not append a fake public migration for incarnation changes. A fresh same-path observation can update the internal current cursor and expose current task attribution from host inventory. Historical binding remains old, and its unavailable portion is explicitly incomplete. A later genuine different-ID move derives its source incarnation from this cursor, not from the last path-only public record or today's source path. The public previous_observed_at chain must retain its existing historical-record semantics.

Source IDs are duplicated in links for relational constraints and validation against immutable JSON. Do not add nullable identity columns without validating their combinations. Native links cannot silently become unknown on read failure. Registry contents are physical identity evidence, not retirement intent. Bounds must cover links/cursors within existing route/binding limits; no unbounded new history collection or per-record Git subprocesses.

## Write registration and transaction boundaries

Create a narrowly scoped internal origin-resolution helper: resolve the command's actual target worktree (which may differ from the caller), verify common/admin/root and reciprocal links, compute incarnation, and bind it to the prepared operation. Recheck under the transition guard and SQLite IMMEDIATE transaction before mutation and before commit, following JournalAdmission's source checks. Reuse observation work where possible; do not weaken generic strict gates globally.

RoutePlanStore::set: validate all input first; resolve request replay before generating a new route/revision. New accepted revision writes registry, route record, origin link and one generation update atomically. A new route after replacement is allowed only against the verified current incarnation. An update to an old route cannot use same path-derived ID plus CAS as authority to change incarnation. Reject implicit rebinding; any explicit cross-worktree retarget already supported by the product needs a precisely scoped target check and retained original start evidence, not incidental path reuse. No new public transfer operation is in this plan.

observe_task_bindings: normalize/validate the inventory, resolve each accepted destination identity, and load the prior origin-aware cursor before applying chronology. Register the target even when no journal exists. Write binding records plus immutable source/destination links when public association ID changes. For same ID with a new incarnation, update only the current cursor/watermark (and generation once), preserving old binding/history. Unknown prior cursor means unknown source; never invent one by probing the current filesystem. The request's source workspace and all observed target workspaces must remain in the authenticated common repository.

Presence/journal must not gain any broader route permission through these helpers. Existing no-route admission is a separate accepted slice. Record-specific route validation should eventually require a capture's route reference to match the intended qualified route identity rather than treating a route ID as current ownership.

## Retries, CAS and observation ordering

- Exact accepted request ID + identical input returns the original result and immutable identity mapping without new registry/link/generation writes. Revalidate repository/store integrity, but do not require its old target to be currently live merely to acknowledge already accepted history. This is receipt replay, not permission to attach or append in a new incarnation.
- Same request ID with changed payload remains a terminal conflict. Missing/corrupt links must not be reconstructed during retry.
- New route request with stale expected revision returns the existing structured revision/current-plan conflict. Identity mismatch must not accidentally produce a new revision after the CAS check.
- Journal old-session retry remains governed by its stricter session-incarnation rule; route receipt policy does not override it.
- Inventory observations older than watermark do not alter cursor/history. Same time plus conflicting identity cannot overwrite the accepted observation. Exact repeated observation remains idempotent. A newer observation of the same ID/same incarnation updates chronology according to current semantics without a false movement record.
- At-least-once transport reconnect must continue replaying the same prepared request bytes. Internal resolved identity must not be rebound midway through one acceptance attempt.

## Read qualification and cache

InputReader pins one SQL generation and loads raw domain bytes, origin links and cursors together. Validate one-to-one link coverage, registry references, ID/JSON agreement, cursor chronology and source-chain consistency before ephemeral projection. Route/binding read errors preserve existing domain warning/error behavior; malformed mandatory identity metadata is never mislabeled ordinary disappearance.

Origin observation should include relevant registry incarnations as well as manifest origins. Frozen legacy inventory/provenance checks remain separate and strict for present old sources. Native registry origins have no fabricated frozen files. Current filesystem identity determines whether each known link matches, is unavailable or was replaced; unknown stays unknown.

Latest route revisions and their start evidence qualify by their own immutable links; never suppress every plan sharing a worktree_id. New route on replacement must remain visible while old route is omitted from live attachment with a warning. Binding source/destination qualify independently. Prefer a private report carrying filtered valid bindings plus complete=false and affected worktree IDs, replacing the current all-or-error overlay limitation without changing public Dock fields. Keep raw cache untouched; requalify live association each read and include relevant origin observations in the existing fingerprint invalidation. No TTL bypass of corruption checks.

## Frozen legacy import and unknown provenance

migration::import maps domain records to uniquely matching manifest worktree origins. This produces frozen_baseline links: a compatibility association anchored at the verified freeze, not proof of physical identity at each earlier event time. Missing origin or ambiguity produces explicit unknown; retain every original JSON record and hash. Binding source and destination may independently be unknown. Imported cursor comes from the retained watermark/latest historical association with the same evidence qualification; do not fabricate missing activity or create journal sessions.

compare_domains/validate_domains must validate expected imported links, coverage, cursor chronology and reference consistency alongside existing data. A native record or schema-1 candidate row lacking trustworthy identity cannot be backfilled from today's same path, even if a journal registry row currently exists. A snapshot proves its exact imported baseline; it does not prove the identity of arbitrary post-activation revisions.

## Schema-1 handling and protected performance evidence

Change store/mod.rs VERSION to 2 and schema.sql accordingly. New/open-existing paths must reject schema 1 with an explicit unsupported-version/recovery error BEFORE any migration/admission mutation. Do not auto-create mandatory tables in version 1 and do not alter version to pretend conversion succeeded. Preserve current safety checks for unknown future versions, missing selectors and fence/provenance loss.

No in-place schema-1 converter in this slice. If conversion is later requested, operate on an independently owned copy, retain original database/backup, validate all existing domain bytes, import only provable links and mark unprovable associations unknown. Activation of a converted copy requires a separately reviewed transition. Existing schema-1 scale corpus and every prior measurement remain frozen and untouched. Build new schema-2 corpus at a new owned path and repeat correctness, cold/warm/query and process acceptance there; prior schema-1 timing is historical evidence only.

## Concrete code locations for implementation ownership

- src/store/schema.sql; src/store/mod.rs VERSION, validate and narrow transaction/admission APIs. New private origin-link module recommended to avoid growing unrelated domains.
- src/route_plan.rs RoutePlanStore::set/build/sql_records: exact receipt and CAS ordering, immutable per-revision links and registration.
- src/journal.rs observe_task_bindings/update_bindings/sql_binding_snapshot and parse_binding_records boundary: keep legacy parser/public history semantics; add SQL identity-aware cursor processing rather than weakening same-ID migration rejection globally.
- src/store/migration.rs import, compare_domains, validate_domains, compare_projection/compare_snapshot: import baseline mapping and shadow validation. Preserve revalidate strict activation checks and source hashes.
- src/store/origin_observation.rs observation union and source validation; src/store/snapshot.rs sql_inputs/qualify_origins/raw cache; src/dock.rs DockStorageInputs private binding report and apply_workspace_evidence completeness handling if necessary. No frontend asset changes.
- src/application.rs existing origin fingerprint seam only if cursor/current-origin observations require it. Runtime wire schemas unchanged; mutation executor should select the new typed admission only for reviewed commands.

## All-table checks and schema-sensitive fixtures to update

Do not claim all-table equality while omitting the three new tables. Extend explicitly maintained snapshot lists in:
- tests/sqlite_origin_lifecycle.rs::sql_snapshot (currently eleven SELECTs; index-based after[7] provenance assertions must become named lookup or be carefully retained).
- tests/sqlite_native_origin_lifecycle.rs::sql_snapshot and tests/application_origin_reanchor.rs::sql_rows.
- tests/shared_runtime_mutations.rs process fault SQL snapshot (currently seven domain tables plus separate generation/derived counters): include all identity links/cursors in fullcommit→kill→restart equality.
- tests/shared_mutation_proxy.rs receipt snapshot and tests/repository_application.rs binding purity snapshot: include corresponding link/cursor state.

Audit injected raw inserts in tests/sqlite_routes.rs, tests/sqlite_bindings.rs and src/store/snapshot.rs tests. Valid corruption fixtures must first create all required identity rows and then independently tamper the target condition; do not disable foreign keys to make impossible states or accidentally test only missing-link rejection. The snapshot unsupported-version test currently writes schema_version=2; after the bump it must use 3 (or another explicitly unsupported version) and retain a separate genuine schema-1 rejection fixture. Inspect src/store/migration.rs domain counts/SQL comparisons and any raw VALUES inserts for column-count assumptions. New side tables keep existing row layouts stable, but coverage checks still need their rows.

Before claiming complete table coverage, compare snapshot-helper table names against sqlite_schema for owned fixture databases, excluding SQLite internal tables; this makes future omissions visible. Do not inspect or modify the preserved performance database for this check.

## Ordered test matrix / acceptance gates

1. Baseline schema-1 concurrency/regression and scoped commit must complete first. Existing native lifecycle RED logs remain preserved.
2. Schema contract: fresh version2 creation, missing mandatory tables/links rejected, wrong pair/null qualification rejected, registry mismatch, unsupported version3, genuine version1 read/write rejection without byte or side-effect changes.
3. Native route/binding-only registration: no journal required; accepted domain record and registry/link/cursor commit together; injected link/cursor write failure rolls back every table and generation.
4. Existing two actual native lifecycle tests become green: post-activation journal-backed and route/binding-only old attachments disappear after same-path recreation; all history/provenance/backup preserved.
5. New route after replacement is visible with its own incarnation; old route remains excluded. CAS/retry: exact old accepted receipt returns identical result without writes; changed request conflicts; stale revision structured payload preserved; old-route implicit incarnation rebinding rejected.
6. Binding chronology: original A then same-path/new-incarnation A observation updates cursor without a public migration; subsequent A→B records source=new A incarnation. Equal-time conflicting identity, older observations, duplicate exact observation, unknown prior source and unavailable destination behave explicitly. Old public JSON chain remains parseable unchanged.
7. Migration: uniquely mapped frozen baseline, missing/ambiguous source unknown, independent source/destination mapping, unchanged JSON/hashes, shadow compare/activation equality, corrupted link/source provenance fails.
8. Read/cache: warm same-generation replacement removes old attachments, new committed links appear immediately, changing identity metadata without generation is detected by existing data_version invalidation; no cross-client attachment leakage. Invalid link cannot be converted to missing-worktree warning.
9. Existing journal/no-route gates, standalone presence/route strict cases, public protocol and mutation/proxy tests, full-upload/no-result-read kill/restart exact retry with full identity-table equality, partial-upload no-effect, reservation/reconnect/time budgets.
10. New owned schema2 scale corpus: cold/warm correctness and bounded process performance, then unchanged UI contract and visible acceptance. Preserve schema1 corpus and negative timing evidence as-is.

Implementation checkpoints: schema+link invariants → native writers/cursor → frozen import/read qualification → runtime/fault/full regression → new scale and UI acceptance. Each production slice gets independent review; no schema2 acceptance claim based solely on old schema1 greens.

## Independent review supplement — required before implementation

Reviewed against current route_plan.rs, journal.rs, store/{mod,startup,migration,snapshot}.rs, executor and application setup. These amendments resolve omissions in the earlier sketch; implementation has not started and this review provides no runtime evidence.

### 1. Watermark without an association is a valid legacy state

journal::legacy_binding_snapshot deliberately preserves a watermark even when no task-bindings.jsonl exists. That tuple contains only host/task/time. It proves no current worktree ID. Amend binding_origin_cursors: current_worktree_id must be nullable, independently of nullable incarnation. Represent three distinct states: known ID plus known incarnation (native_verified/frozen_baseline); known ID plus unknown incarnation (unknown); no known association (both NULL, distinct unobserved qualification, history_observation_id NULL). The last state must still retain its watermark and chronology. Do not infer a worktree from today's inventory, a registry entry or the caller to fill the missing ID.

A newer first actual association after an unobserved cursor produces task_association_observed with from_worktree_id and previous_observed_at both NULL, preserving the public parser's history semantics. An older/equal observation remains blocked by the retained watermark. Preserve original imported timestamp strings and JSON bytes; compare RFC3339 instants for chronology. Do not rewrite a valid legacy +00:00 spelling to Z merely because the cursor requirement used the word canonical. Cursor observed_at must agree with the retained watermark representation.

### 2. Typed admission must cross every existing strict boundary

A helper called only inside route_plan::set or update_bindings is too late. Today executor maps route mutations to StartupAdmission::Strict, which calls prepare_first_write, which calls strict active_existing. Then domain_write_guarded again runs check_legacy_drift before invoking the closure. Application inventory acceptance independently calls prepare_first_write before observe_task_bindings. These gates will reject old receipt replay and a legitimate new route/binding after origin replacement before the typed helper can act.

Add narrow typed backend-selection and transactional admission seams in startup.rs, store/mod.rs, mutation startup selection, executor and application inventory. An already-active backend may validate selector/fence/provenance without requiring every old origin live solely to select the backend; actual record-specific admission then validates all relevant immutable links and present frozen source bytes. Legacy/fresh activation keeps its existing strict freeze rules. Generic domain_write, standalone presence and route-bearing capture must retain their strict policy until explicitly qualified; do not reuse the no-route journal exception as blanket route/binding authority.

Receipt lookup and stale-CAS conflict must run against complete validated raw route history before requiring the old target live. An exact receipt cannot be extracted from the filtered live route list, and integrity errors in links remain errors even for replay. The pre-commit check must not reintroduce a generic all-origins-live condition that defeats the typed admission.

### 3. Preserve absent-worktree edits and explicit retarget semantics

Current RoutePlanStore::build allows a revision when its previous plan has the same worktree_id even if that worktree is absent. This permits abandoning or editing historical intent. It also supports an explicit worktree-ID retarget while preserving the route's initial start_commit.

Use these cases explicitly:
- New route: require verified current target incarnation and register/link it atomically. A new route at reused ID A may target A2 without making an older A1 route current.
- Existing route, unchanged worktree ID: inherit the preceding revision's immutable identity qualification. An absent target can still receive an abandonment/intent revision; an existing replacement must never silently promote the inherited A1 link to A2. Such a historical revision remains excluded from live attachment. Unknown identity remains unknown. Live execution/capture permission is separate from editing history.
- Existing route, explicitly different worktree ID: retain the product's retarget path, require a verified live new destination, link the new revision to that destination, preserve the original start_commit and first-record start evidence. Do not require an unavailable original source to reappear merely to retarget intent; do not relabel its original start evidence as proof from the new destination.
- Exact replay precedes live-target resolution and returns its original result; changed request payload conflicts. Stale expected_revision returns the existing structured current-plan/revision conflict before evaluating whether a new revision could be admitted. Neither branch inserts a registry row or changes generation.

Add explicit absent-old-target abandonment, same-ID replacement historical edit (no rebind), A-to-B retarget after A removal, stale-CAS after replacement, and old-receipt replay after removal tests. Keep capture validation strict and record-qualified; accepting an intent revision does not authorize capture on a replaced route.

### 4. Binding cursor and immutable public chain are different evidence

For A1 association at t1, same-ID A2 observation at t2, then A-to-B movement at t3: the new public record's previous_observed_at remains t1 and from_worktree_id remains A, while its immutable source identity link is A2 from the accepted cursor. A read validator must not require that link to equal the previous public record's destination incarnation A1. That would reject the very same-path/new-incarnation case the cursor exists to represent. The cursor is overwritten to B only after creating the t3 record/link atomically. Validate the source identity captured by that transaction without fabricating an extra public t2 migration.

Do not leave the current update_bindings same-ID early continue in front of SQL cursor processing. Separate SQL identity-aware processing from the legacy helper, retain max-record/byte bounds and increment generation exactly once even when only a cursor/watermark changed. Scope source-chain checks to ID/public-time continuity plus valid qualified links, with the cursor transition rule explicitly tested.

### 5. Read, version and recovery coverage omissions

Keep raw domain history available to receipt/CAS, migration and audit readers; qualify a copy for live Dock attachment. Audit direct DockService list_with_starts/read_task_bindings callers as well as InputReader so a legacy presentation path cannot bypass per-record identity qualification. Full historical getters must not be accidentally redefined as filtered live history.

Add src/store/startup.rs::validate_shadow to the schema-sensitive table list: its existing exact-empty loop enumerates ten tables and otherwise could overlook extra schema2 link/cursor rows during owned-empty recovery. Add all new tables and their correct empty/imported expectations. The existing frozen compare helpers must distinguish imported baseline link expectations from subsequently accepted native links rather than recomputing all link qualifications from today's manifest.

Version rejection must precede creation of a new transition directory/lock or initialization/WAL admission side effects where the plan promises no effects. RepositoryStore::open currently acquires the transition guard before examining the database version. A read-only early version check plus under-guard recheck (or an equivalent checked existing-store path) is required; simply bumping VERSION does not establish the no-side-effect schema1 rejection claim. Include a valid schema1 fixture without a pre-existing transition directory and compare its full directory inventory before/after attempted open/write. Preserve the original corpus without querying it through mutation-capable paths.

Required review verdict: feasible after these amendments. The nullable association state and typed outer admission are implementation blockers in the unamended design; absence/retarget, binding-chain, startup all-table and side-effect-free version handling need explicit tests before claiming compatibility.

Root integration clarification: version rejection must not create a transition
directory, change domain/schema data, activate, freeze or repair a store. As in
the global plan, SQLite-managed WAL/SHM bookkeeping from a checked read-only
connection to an existing live database remains permitted and must be reported
separately from domain mutation. Do not use immutable mode on a mutable store
to obtain a misleading byte-for-byte sidecar assertion.
