# Phase 1B final-review fix report

Base: `4f42d1107d889bcfc4f1b21a42358420bba87607`

## Outcome

The whole-branch final-review findings were resolved as one cohesive Phase 1B
hardening pass. Native Codex/Claude hooks now record only bounded lifecycle and
activity facts, semantic capture goes through the shared MCP/Capture Kernel, all
three current adapters derive an honest effective Grade D from their implemented
capabilities, and adapter configuration changes require a reviewed digest followed
by a locked compare-and-swap transaction. No source Git operation, remote access,
HTTP listener, daemon, or user-owned `work/` path was added.

## Finding closure

### 1. Capability truth and semantic routing

Implementation:

- `CaptureCapabilities::grade` remains the single A/B/C/D calculation. New
  `host_capabilities` snapshots describe only observable behavior. Native hooks do
  not advertise mutation, Evidence, tool-result, workspace-rebind, pre-mutation
  blocking, or commit-mapping capabilities. Generic MCP advertises its explicit
  semantic tools but no mutation/diff acknowledgement, so it also derives Grade D.
- Hook and MCP event payloads use the derived grade rather than A/C literals.
- Verification reports `configured` separately from `activation_verified`.
  Missing executable reachability, native-host trust/managed policy, and Generic
  MCP host registration are explicit unresolved activation reasons.
- Unqualified `adapter verify` checks Codex, Claude, and Generic MCP.
- Native prompt hooks emit content-digest activity only. Requirement, Decision, and
  Evidence records are entered through MCP and the shared `CaptureKernel`.

Tests/evidence:

- RED tests rejected hard-coded optimistic grades and capabilities, then GREEN in
  `effective_host_capabilities_are_derived_and_honestly_grade_d`,
  `generic_context_reports_capability_derived_grade_d`, and
  `verification_reports_installed_capability_not_requested_grade`.
- `generic_descriptor_does_not_claim_unobservable_host_registration` was RED while
  Generic MCP could report activation from a descriptor plus PATH alone, then GREEN
  after registration became an explicit unresolved reason.
- `installed_native_bindings_and_real_mcp_share_a_truthful_canonical_contract`
  proves native activity and real MCP semantic records coexist in one journal.

### 2. Privacy, lifecycle, provenance, and mutation truth

Implementation:

- Native input normalization is an allowlist: bounded identifier-shaped IDs,
  enumerated status values, booleans, and hashes only. Transcript paths, cwd,
  prompts, commands, patches, tool input/output, assistant messages, compaction
  bodies, unknown nested metadata, and arbitrary status text never enter the event
  payload. The complete serialized event envelope is limited to 64 KiB.
- `Stop` maps to the new canonical `turn_completed`; only `SessionEnd` maps to
  `session_stopped`.
- Missing Subagent parent IDs derive as `<host>:<session>` while supplied valid
  parent IDs are retained.
- Stable IDs use explicit event/hook IDs first, then real tool-use, turn/prompt, or
  agent IDs. Repeated status values such as session-end `reason` are not treated as
  unique identifiers, and identifiable retries are idempotent.
- CLI `--event` is checked against the canonical payload `hook_event_name`.
  Mismatches and unsupported names become bounded gaps without echoing attacker
  text.
- A completed write-capable tool records `tool_completed` plus a
  `mutation_unverified` gap. Phase 1B never emits a guessed `mutation_observed` or
  a dirty-tree target derived from HEAD.

Tests/evidence:

- RED canary tests exposed recursively copied host content and identifier-shaped
  free text; GREEN coverage is in
  `native_content_and_unknown_recursive_metadata_never_cross_the_allowlist`,
  `invalid_identifier_shaped_native_fields_do_not_cross_the_allowlist`, and every
  pinned fixture scenario.
- Lifecycle, parent, mismatch, retry, and mutation tests are
  `stop_is_turn_completion_and_session_end_alone_stops_the_session`,
  `supplied_parent_is_preserved_and_absent_parent_is_derived`,
  `identifiable_retried_hooks_are_idempotent_and_event_name_mismatch_is_a_gap`, and
  `write_capable_tool_name_yields_activity_and_an_unverified_gap_only`.
- `status_values_do_not_collide_and_explicit_event_ids_are_stable` was RED because
  identical status reasons collided, then GREEN after ID derivation was restricted
  to identifiers.

### 3. Host conformance

Implementation:

- Pinned event-specific Codex and Claude payloads come from their documented hook
  schemas and contain privacy canaries in every content-bearing field. Sources and
  the pinning rationale are recorded beside the fixtures.
- Conformance installs a reviewed native binding, executes the compiled command
  from that binding for lifecycle/activity, and invokes the real MCP stdio process
  for Requirement, Decision, and Evidence.
- The compared canonical contract retains event type, route, actor/parent,
  sequence, grade, semantic evidence, and native-versus-semantic route role.

Tests/evidence:

- `pinned_official_event_fixtures_have_the_same_truthful_lifecycle_contract` covers
  all ten installed native events for both hosts.
- `installed_native_bindings_and_real_mcp_share_a_truthful_canonical_contract`
  covers compiled commands, retry idempotence, privacy canaries, turn/session
  distinction, parent derivation, Grade D, gaps, and MCP semantic evidence.
- `installed_binding_turns_an_event_name_mismatch_into_an_honest_gap` covers the
  executable installed-binding mismatch path.

Fixture sources checked for this pass:

- `https://developers.openai.com/codex/hooks`
- `https://code.claude.com/docs/en/hooks`

### 4. Journal isolation, readers, and durability

Implementation:

- `CaptureKernel::new` is fallible and rejects a context whose session differs
  from its `JournalStore`. Journal append and durable-intent recovery independently
  enforce the same invariant.
- Public replay takes the session lock, completes or cleans durable intent, performs
  full canonical/hash-chain validation, and refreshes the derived index. Locked
  internal replay prevents self-deadlock.
- Intent, append, index, rename, removal, and new-directory boundaries are synced as
  supported. File/directory helpers reject symlinks and Windows reparse points,
  compare opened-handle identities with named paths, and revalidate the stored
  session/root identities.
- The hot append path uses a bounded, canonical, validated tail index containing
  journal byte count, record count, last hashed record, and a Bloom membership
  hint. Any potential retry falls back to full replay. Public replay still
  validates all earlier journal bytes, and the existing status integrity path is
  unchanged, so the optimization does not turn the index into an authority source.

Tests/evidence:

- RED/GREEN isolation coverage:
  `capture_kernel_rejects_a_journal_for_another_session` and
  `append_rejects_an_event_for_a_different_session_directory`.
- Crash/read concurrency coverage:
  `replay_recovers_a_durable_intent_immediately_after_a_crash` and
  `public_replay_waits_for_the_writer_lock_before_reading`.
- Identity/no-follow coverage:
  `replacing_an_open_session_directory_is_refused_without_writing_the_replacement`
  and the Unix symlinked-session regression, plus existing torn-tail, duplicate,
  canonicalization, and tamper replay tests.

### 5. MCP negotiation and current-schema validation

Implementation:

- Modern discovery and `-32022` data advertise only `2026-07-28`. Legacy
  `2025-11-25` is available only after initialize negotiation. An unsupported
  legacy initialize receives a successful supported-version counteroffer.
- Modern `_meta` is total-size bounded and schema-equivalent: protocol version and
  client capabilities are required; `clientInfo` is optional but fully validated
  when present; nested sampling/elicitation objects, experimental/extension object
  maps, mandatory extension prefixes, and Implementation icon fields are checked.
- Newline framing drains an oversized line before processing the next line.
  Notifications remain silent, stdout remains JSON-only, explicit IDs remain
  deterministic, RFC3339 is enforced, and concurrent MCP processes serialize
  journal appends.

Tests/evidence:

- Negotiation RED/GREEN:
  `modern_discovery_and_version_errors_never_advertise_legacy_versions` and
  `unsupported_legacy_initialize_is_a_successful_counteroffer`.
- Metadata/schema RED/GREEN:
  `modern_metadata_validates_client_identity_and_nested_capability_shapes`; its
  optional-client assertion first failed while `clientInfo` was incorrectly
  required, and malformed icon/extension cases failed open before the validator
  was completed.
- Framing and bounds:
  `mcp_line_limit_is_enforced_before_json_parsing_and_next_line_still_works` and
  `modern_metadata_and_semantic_arguments_have_independent_bounds`.
- The full `mcp_stdio` suite preserves legacy/modern coexistence, notification
  silence, JSON-RPC IDs/errors, stdout purity, deterministic capture IDs, and
  process concurrency.

### 6. Installer transaction safety

Implementation:

- `adapter plan` produces `sha256-...` over action, host, source directory identity,
  project-relative target, exact prior bytes/absence, target identity/mode, parent
  identity/absence, and exact desired bytes/removal.
- Install and uninstall require that reviewed token. A repository-local lock
  serializes installers; the plan is rebuilt after locking and the snapshot is
  compared again immediately before mutation.
- Existing files are replaced with no-replace/atomic primitives and an exact
  recovery backup. Windows uses `ReplaceFileW`/write-through moves and verifies the
  named result before backup cleanup. Linux uses `renameat2(RENAME_NOREPLACE)`.
  Other platforms fail closed for replacement/removal. Unix modes are preserved;
  Windows replacement does not rewrite ACLs using a weaker portable permission
  abstraction.
- Source, parent, target, temp, and backup paths receive no-follow/reparse and
  identity checks. Stale DevMap transaction artifacts are preserved and block the
  operation instead of being overwritten.
- Generic verification treats valid descriptor drift as `modified` Grade D, while
  malformed JSON/shape remains an error.

Tests/evidence:

- Approval/digest and CAS RED/GREEN:
  `install_requires_the_exact_reviewed_plan_token`,
  `plan_digest_covers_exact_prior_bytes_identity_host_and_desired_result`,
  `a_user_edit_between_plan_and_install_wins_the_compare_and_swap`, and
  `a_user_edit_after_uninstall_review_is_never_removed`.
- Races/identity:
  `two_installers_from_one_plan_cannot_both_commit`,
  `replacing_a_target_with_identical_bytes_still_invalidates_the_plan_identity`,
  and `replacing_the_adapter_parent_directory_invalidates_the_reviewed_plan`.
- Mode, symlink/reparse, stale-artifact, readback rollback, and backup preservation
  are covered by platform-gated integration/unit tests in `adapter_install`,
  `final_review_installer`, and the `adapter` unit module.
- `generic_recognized_drift_is_a_modified_grade_d_report_but_bad_shape_is_malformed`
  covers the Generic descriptor distinction.

### 7. Resource limits and latency

Implementation:

- Hook bodies (1 MiB) and MCP lines (1 MiB) are limited before JSON parsing. MCP
  metadata (64 KiB), arguments (256 KiB), semantic strings (16 KiB), arrays (64
  items plus aggregate bound), complete events (64 KiB), journal records, durable
  intent, derived index, sessions (100,000 records), and journals (64 MiB) are
  bounded with typed errors or structured tool errors.
- A running MCP process resolves/caches its stable `SourceWorkspace` once.
- The durable validated tail index removes full-journal replay from normal append;
  the long-session gate exercises 1,000 preload appends, a 64-append steady-state
  window, a compiled native hook after that history, and final full replay.

Tests/evidence:

- Resource-limit RED/GREEN tests live in `final_review_capture`,
  `final_review_journal`, and `final_review_mcp`. The semantic-oversize test was RED
  with an untyped domain error before returning `ResourceLimit`.
- Release performance evidence is recorded in the Verification section below.

### 8. Domain validation

Implementation and evidence:

- Native timestamp candidates are accepted only when RFC3339; invalid values use a
  valid receipt timestamp (`invalid_native_timestamp_falls_back_to_a_valid_receipt_time`).
- A context HEAD, when present, must be lower-case 40- or 64-hex
  (`context_head_must_be_a_lowercase_sha1_or_sha256`). Deserialization routes back
  through the same validated constructors.

## Interface and migration notes

- `CaptureKernel::new(...)` now returns `Result<CaptureKernel, DevMapError>`.
- Library adapter mutation is now plan-based:
  `install_adapter(AdapterPlan, token)` and
  `uninstall_adapter(AdapterPlan, token)`. Removal plans are created with
  `plan_uninstall_adapter`.
- CLI install/uninstall require `--plan-digest`; removal preview is
  `adapter plan --action uninstall`.
- `adapter verify --host` remains optional and now means all three hosts when
  omitted. Output adds `configured`, `activation_verified`, and activation reasons.
- `EventType::TurnCompleted` / `turn_completed` is the canonical Stop event.
- The Generic convenience functions remain aliases to the unified adapter surface,
  but unsafe token-free mutation signatures were intentionally not retained.

## Verification

The brief's exact gate was executed from this worktree after the final source edits
and report draft. Every command passed:

```text
cargo fmt --all -- --check                                  PASS
cargo clippy --all-targets --all-features -- -D warnings   PASS
cargo test --all-targets --all-features                    PASS (131 passed, 0 failed)
cargo build --release --all-features                       PASS
target/release/devmap adapter --help                       PASS
target/release/devmap hook --help                          PASS
target/release/devmap mcp --help                           PASS
git diff --check                                           PASS
```

The independent release-mode long-session gate also passed:

```text
cargo test --release --test long_session_performance -- --nocapture
records=1065
preload_ms=21045                 threshold=60000
sample_count=64
sample_total_ms=1203             threshold=20000
steady_state_p95_ms=23           threshold=750
native_hook_ms=365               threshold=5000
result=PASS
```

After recording these results, the formatting and whitespace checks were repeated
to cover the report-only edit.

## Residual limitations

- Effective Capture Grade remains D by design. Phase 1B has no mutation ledger,
  before/after dirty-tree proof, evidence-to-mutation association, commit mapping,
  tool-result semantics, or workspace rebind. Those are not simulated here.
- Native prompt content is not semantic evidence; only its digest and activity fact
  are retained. Agents must use MCP for explicit Requirement/Decision/Evidence.
- DevMap cannot prove native-host trust/managed-policy approval or Generic MCP host
  registration from project files, so those remain visible unresolved activation
  reasons.
- External editors do not participate in DevMap's installer lock. Exact bytes and
  identities are checked after lock acquisition and again at the commit boundary;
  detected late changes fail closed and recovery backups are preserved when cleanup
  is unsafe. An adversarial process that starts writing after the final OS CAS is
  outside a cooperative cross-process lease.
- Directory `fsync` is used on Unix. Windows uses handle identity checks and
  write-through atomic APIs; directory sync itself is not exposed by Rust's
  portable file API.
- Existing-file replacement/removal is implemented only where the required safe OS
  primitive is available (Windows and Linux); unsupported targets return an error.
- The append index validates length and the hashed tail for low latency. A deliberate
  earlier-record rewrite is detected by public replay, not by every append; the
  existing Context Repository status checks are unchanged.

## Changed files

Runtime and interfaces:

- `Cargo.toml`, `Cargo.lock`
- `src/adapter.rs`, `src/fs_security.rs`
- `src/capture.rs`, `src/events.rs`, `src/hook.rs`, `src/journal.rs`, `src/mcp.rs`
- `src/cli.rs`, `src/commands.rs`, `src/error.rs`, `src/lib.rs`

Tests and fixtures:

- `tests/final_review_capture.rs`, `tests/final_review_installer.rs`
- `tests/final_review_journal.rs`, `tests/final_review_mcp.rs`
- `tests/long_session_performance.rs`
- `tests/adapter_conformance.rs`, `tests/adapter_install.rs`
- `tests/capture_domain.rs`, `tests/hook_flow.rs`, `tests/journal_flow.rs`
- `tests/mcp_stdio.rs`, `tests/phase_1b_acceptance.rs`, `tests/support/mod.rs`
- `tests/fixtures/hooks/codex-events.json`
- `tests/fixtures/hooks/claude-events.json`
- `tests/fixtures/hooks/README.md`

Documentation:

- `README.md`, `README.zh-CN.md`
- `docs/ai-development-map-requirements.md`
- `docs/superpowers/specs/2026-08-27-git-workflow-orchestrator-design.md`
- `.superpowers/sdd/2026-08-27-devmap-phase-1b-native-capture/final-review-fix-report.md`
