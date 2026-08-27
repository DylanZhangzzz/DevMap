# DevMap Phase 1B Native Capture Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver a host-neutral capture kernel with project-local Codex, Claude, and Generic MCP adapters that write verifiable local evidence journals without changing source Git history.

**Architecture:** Thin host adapters normalize native lifecycle input into a canonical Event Envelope. The Capture Kernel validates structured Requirement, Decision, Evidence, and Gap records; an append-only per-worktree Journal persists them. Project-local installer code merges only DevMap-owned hook configuration. A Generic MCP stdio endpoint exposes the same recording operations when native hooks are unavailable.

**Tech Stack:** Rust 1.96, Cargo, clap 4, serde, serde_json, sha2, time, thiserror, tempfile, and the system Git executable. No database, daemon, Node runtime, Git host API, custom Git refs, or Git Notes.

**Spec:** `docs/superpowers/specs/2026-08-27-git-workflow-orchestrator-design.md`

## Global Constraints

- Phase 1B may create or merge only explicitly approved project-local adapter configuration files.
- Phase 1B must not create a branch or worktree, stage, commit, push, rebase, stash, reset, alter refs, or alter source Git configuration.
- Do not reconstruct pre-adoption decisions or infer decision semantics from a code diff.
- A human instruction is a Requirement Trace; it is never silently recast as an Agent Decision.
- An Agent Decision requires authority, rationale, alternatives, scope, and revisit trigger.
- A missing semantic record creates `capture.gap`; never fabricate one later.
- All source-repository Git reads use argument arrays, never shell interpolation.
- All local journal records are deterministic JSON with SHA-256 integrity linkage.
- The installer is preview-first, idempotent, preserves unrelated Hook entries, and uninstalls only its own binding IDs.
- Raw transcripts are off by default; canonical output retains structured traces and approved quotations only.
- `work/` is user-owned and must never be modified, staged, or committed.
- Every behavior change begins with a failing automated test.

---

## Target file layout

```text
Cargo.toml
README.md
README.zh-CN.md
src/
  adapter.rs             # Host matrix, config planning, merge, verify, uninstall
  canonical.rs
  capture.rs             # Requirement/Decision/Evidence/Gap domain validation
  cli.rs                 # adapter, hook, and mcp subcommands
  commands.rs            # Existing Phase 1A commands only
  context.rs
  error.rs
  events.rs              # Canonical Event Envelope and capability handshake
  git.rs                 # Read-only source workspace discovery
  hook.rs                # JSON stdin normalization and local capture dispatch
  journal.rs             # Per-worktree append-only hash-chain journal
  lib.rs
  main.rs
  mcp.rs                 # Generic stdio JSON-RPC capture server
tests/
  adapter_conformance.rs
  adapter_install.rs
  capture_domain.rs
  hook_flow.rs
  journal_flow.rs
  mcp_stdio.rs
  phase_1b_acceptance.rs
  support/mod.rs
```

## Task 1: Add capture CLI surface and typed error boundary

**Files:**

- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Create: `tests/capture_domain.rs`
- Modify: `tests/cli_help.rs`

**Consumes:** existing `Cli`, `Command`, `DevMapError`, and `run` in `src/lib.rs`.

**Produces:** all later tasks use these exact command types and dispatch targets.

```rust
pub enum Command {
    Init(InitArgs),
    CommonGround { command: CommonGroundCommand },
    Status(StatusArgs),
    Adapter { command: AdapterCommand },
    Hook { command: HookCommand },
    Mcp(McpArgs),
}

pub enum AdapterCommand {
    Plan(AdapterPlanArgs),
    Install(AdapterInstallArgs),
    Verify(AdapterVerifyArgs),
    Uninstall(AdapterUninstallArgs),
}

pub enum HookCommand {
    Handle(HookHandleArgs),
}

pub enum AdapterHost {
    Codex,
    Claude,
    GenericMcp,
}
```

- [ ] **Step 1: Write failing help tests.**

```rust
#[test]
fn help_exposes_phase_1b_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_devmap"))
        .args(["adapter", "--help"])
        .output()
        .unwrap();
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(stdout.contains("plan"));
    assert!(stdout.contains("install"));
    assert!(stdout.contains("verify"));
    assert!(stdout.contains("uninstall"));
}
```

- [ ] **Step 2: Run the focused test and confirm failure.**

Run: `cargo test --test cli_help help_exposes_phase_1b_commands -- --exact`

Expected: FAIL because `adapter` is absent from the parser.

- [ ] **Step 3: Add parser types and typed unsupported dispatch.**

Add `Adapter`, `Hook`, and `Mcp` to `Command`; parse `--source PATH` for every Phase 1B command. Add typed `DevMapError` variants for malformed adapter config, unsupported host events, invalid event envelopes, journal corruption, duplicate sequence, and unsafe installer overwrite. Dispatch to temporarily typed command functions, not `panic!`.

- [ ] **Step 4: Run focused parser and existing CLI tests.**

Run: `cargo test --test cli_help`

Expected: PASS; Phase 1A command help remains present.

- [ ] **Step 5: Commit Task 1.**

```bash
git add src/cli.rs src/lib.rs src/error.rs tests/cli_help.rs tests/capture_domain.rs
git commit -m "[FEAT](capture): add adapter command surface"
```

## Task 2: Define canonical events and capability handshake

**Files:**

- Create: `src/events.rs`
- Modify: `src/lib.rs`
- Modify: `src/canonical.rs`
- Modify: `tests/capture_domain.rs`

**Consumes:** `canonical_json`, `sha256_hex`, and typed errors from Task 1.

**Produces:** `EventEnvelope`, `EventType`, `CaptureCapabilities`, and `CaptureGrade` used by Journal, Kernel, Hook, Adapter, and MCP code.

```rust
pub const EVENT_SCHEMA_VERSION: &str = "devmap/event/1";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventType {
    SessionStarted, SessionStopped, InstructionObserved, AgentStarted,
    AgentStopped, ToolRequested, ToolCompleted, MutationObserved,
    DecisionRecorded, EvidenceRecorded, ContextCompacting, ContextCompacted,
    GitActionProposed, GitActionAuthorized, GitActionExecuted,
    GitActionFailed, AuthorityChanged, CaptureGap,
}

pub struct EventEnvelope {
    pub schema_version: String,
    pub event_id: String,
    pub event_type: EventType,
    pub sequence: u64,
    pub occurred_at: String,
    pub host: HostIdentity,
    pub actor: ActorIdentity,
    pub context: SessionContext,
    pub payload: serde_json::Value,
}

pub struct CaptureCapabilities {
    pub lifecycle_events: Vec<EventType>,
    pub pre_mutation_blocking: bool,
    pub subagent_lifecycle: bool,
    pub workspace_rebind: bool,
    pub tool_results: bool,
    pub commit_mapping: bool,
    pub raw_transcript: bool,
}

pub enum CaptureGrade { A, B, C, D }
```

- [ ] **Step 1: Write failing domain tests.**

Test that an envelope rejects a blank ID, sequence `0`, blank source repository, blank session ID, unsupported schema version, upper-case SHA-like IDs where a lower-case ID is required, and non-object payloads for structured events. Test that `CaptureCapabilities::grade()` returns A only when lifecycle, mutation, evidence, commit mapping, and subagent coverage are present; prompt-only capability returns D.

- [ ] **Step 2: Run the new tests and confirm failure.**

Run: `cargo test --test capture_domain event_envelope -- --nocapture`

Expected: FAIL because `events` does not exist.

- [ ] **Step 3: Implement validated event construction.**

Implement constructors rather than public unchecked constructors. Serialize event names in `snake_case`; serialize grades as `A`, `B`, `C`, `D`. Add `EventEnvelope::canonical_bytes()` and `EventEnvelope::sha256()` using existing canonical encoding. Reject floats anywhere in the event payload by walking the `serde_json::Value` before canonicalization.

- [ ] **Step 4: Add capability-grade matrix tests.**

Use a table test for Codex-native, Claude-native, Generic MCP, and prompt-only capabilities. Assert that a missing `tool_results` or `commit_mapping` can never report Grade A.

- [ ] **Step 5: Run focused tests.**

Run: `cargo test --test capture_domain`

Expected: PASS.

- [ ] **Step 6: Commit Task 2.**

```bash
git add src/events.rs src/lib.rs src/canonical.rs tests/capture_domain.rs
git commit -m "[FEAT](capture): define canonical lifecycle events"
```

## Task 3: Add per-worktree append-only journal with integrity checks

**Files:**

- Create: `src/journal.rs`
- Modify: `src/git.rs`
- Modify: `src/lib.rs`
- Create: `tests/journal_flow.rs`
- Modify: `tests/support/mod.rs`

**Consumes:** `EventEnvelope` and canonical SHA-256 encoding from Task 2.

**Produces:** `SourceWorkspace`, `JournalStore`, and `JournalRecord` for capture persistence and recovery.

```rust
pub struct SourceWorkspace {
    pub root: PathBuf,
    pub git_dir: PathBuf,
    pub branch: Option<String>,
    pub head: String,
}

pub struct JournalStore {
    root: PathBuf,
    session_id: String,
}

pub struct JournalRecord {
    pub sequence: u64,
    pub event: EventEnvelope,
    pub previous_sha256: Option<String>,
    pub sha256: String,
}

impl JournalStore {
    pub fn open(workspace: &SourceWorkspace, session_id: &str) -> Result<Self, DevMapError>;
    pub fn append(&self, event: EventEnvelope) -> Result<JournalRecord, DevMapError>;
    pub fn replay(&self) -> Result<Vec<JournalRecord>, DevMapError>;
}
```

- [ ] **Step 1: Extend test support with linked-worktree fixtures.**

Add `linked_worktree(repo: &Path, branch: &str) -> TempDir` that creates a branch and `git worktree add` fixture. Keep fixture Git operations explicit and local.

- [ ] **Step 2: Write failing journal tests.**

Cover: journal root resolves under `git rev-parse --git-dir`; two linked worktrees get different journal roots; records append in order; `previous_sha256` forms a chain; replay rejects a modified record, duplicate sequence, skipped sequence, and malformed JSON; source HEAD, index, branch, refs, and config are unchanged after `open`, `append`, and `replay`.

- [ ] **Step 3: Run the test and confirm failure.**

Run: `cargo test --test journal_flow`

Expected: FAIL because `SourceWorkspace` and `JournalStore` are absent.

- [ ] **Step 4: Implement read-only workspace discovery.**

Add `SourceGitInspector::workspace()` using only:

```text
git rev-parse --show-toplevel
git rev-parse --git-dir
git rev-parse HEAD
git symbolic-ref --short -q HEAD
```

Resolve a relative Git dir against the source root. Do not call `git config`, `git update-ref`, `git add`, or any write command.

- [ ] **Step 5: Implement the journal format.**

Create `<git-dir>/devmap/sessions/<session-id>/events.ndjson`. Each line is canonical JSON for `JournalRecord` plus one terminating newline. Before appending, replay and verify the existing chain. Open append mode, write one complete record, then `sync_data`. Reject an event whose sequence is not exactly the next expected sequence or whose `event_id` already appears.

- [ ] **Step 6: Run focused tests.**

Run: `cargo test --test journal_flow`

Expected: PASS.

- [ ] **Step 7: Commit Task 3.**

```bash
git add src/git.rs src/journal.rs src/lib.rs tests/journal_flow.rs tests/support/mod.rs
git commit -m "[FEAT](capture): persist per-worktree event journals"
```

## Task 4: Implement Capture Kernel semantic records and gap detection

**Files:**

- Create: `src/capture.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Modify: `tests/capture_domain.rs`

**Consumes:** `EventEnvelope`, `JournalStore`, and `CaptureGrade`.

**Produces:** validated structured payloads and `CaptureKernel::record_*` functions used by Hook and MCP handlers.

```rust
pub struct RequirementTraceInput {
    pub source_kind: String,
    pub source_locator: Option<String>,
    pub quoted_text: String,
}

pub struct AgentDecisionInput {
    pub decision: String,
    pub basis: Vec<String>,
    pub alternatives: Vec<String>,
    pub rationale: String,
    pub scope: String,
    pub authority: String,
    pub revisit_trigger: String,
}

pub struct EvidenceInput {
    pub kind: String,
    pub target: String,
    pub command: Option<String>,
    pub outcome: String,
}

pub struct CaptureKernel;
impl CaptureKernel {
    pub fn record_requirement(... ) -> Result<EventEnvelope, DevMapError>;
    pub fn record_decision(... ) -> Result<EventEnvelope, DevMapError>;
    pub fn record_evidence(... ) -> Result<EventEnvelope, DevMapError>;
    pub fn record_gap(... ) -> Result<EventEnvelope, DevMapError>;
}
```

- [ ] **Step 1: Write failing classification tests.**

Test that a human-provided quote produces `instruction_observed`/Requirement Trace only; it cannot create an Agent Decision. Test that an Agent Decision rejects blank authority, rationale, scope, or revisit trigger, and rejects an empty alternatives list for a material route. Test that a mutation event with no associated Requirement or Decision creates `capture_gap` instead of generating a guessed reason.

- [ ] **Step 2: Run the tests and confirm failure.**

Run: `cargo test --test capture_domain kernel -- --nocapture`

Expected: FAIL because `CaptureKernel` is absent.

- [ ] **Step 3: Implement payload validators and redaction boundary.**

Store a structured source reference and supplied approved quotation. Do not copy raw prompt text into an event unless the input explicitly sets `raw_transcript_opt_in=true`; reject that flag in Phase 1B with a typed `RawTranscriptDisabled` error. Validate evidence targets as `commit:<sha>`, `artifact:<sha>`, or `workspace:<fingerprint>`; a Phase 1B workspace target must remain provisional.

- [ ] **Step 4: Implement kernel-to-journal recording.**

The kernel obtains the next journal sequence, creates a validated envelope, appends it, and returns the immutable `JournalRecord`. It does not inspect code diffs or run Git write commands.

- [ ] **Step 5: Run domain and journal regression tests.**

Run: `cargo test --test capture_domain --test journal_flow`

Expected: PASS.

- [ ] **Step 6: Commit Task 4.**

```bash
git add src/capture.rs src/lib.rs src/error.rs tests/capture_domain.rs
git commit -m "[FEAT](kernel): record traceable capture semantics"
```

## Task 5: Implement JSON hook normalization and status-safe responses

**Files:**

- Create: `src/hook.rs`
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Create: `tests/hook_flow.rs`

**Consumes:** Host enum, Event Envelope, Capture Kernel, Journal Store, and Source Workspace.

**Produces:** `devmap hook handle --host HOST --event EVENT --source PATH` handler usable by Codex and Claude command Hooks.

```rust
pub fn handle_hook(
    args: HookHandleArgs,
    stdin: &mut dyn std::io::Read,
) -> Result<CommandOutput, DevMapError>;

pub fn normalize_hook_input(
    host: AdapterHost,
    event: &str,
    input: serde_json::Value,
    workspace: &SourceWorkspace,
) -> Result<Vec<EventEnvelope>, DevMapError>;
```

- [ ] **Step 1: Write failing hook tests using real JSON stdin fixtures.**

Cover Codex `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PreCompact`, `PostCompact`, `SubagentStart`, `SubagentStop`, `Stop`, and `SessionEnd`; cover Claude counterparts. Verify all normalize to the same event types for an equivalent scenario. Verify unknown input fields are retained only under a bounded `host_metadata` object, while missing mandatory context becomes `capture_gap`, not a crash.

- [ ] **Step 2: Run the hook tests and confirm failure.**

Run: `cargo test --test hook_flow`

Expected: FAIL because the hook handler is absent.

- [ ] **Step 3: Implement stdin parsing and no-op-safe behavior.**

Read one JSON object from stdin. A malformed input returns a non-zero typed error without writing a journal entry. A valid unrecognized event writes a `capture_gap` with `reason=unsupported_host_event` and returns JSON that permits the host to continue. Never make a hook failure claim that a blocked operation was captured.

- [ ] **Step 4: Implement host maps.**

Codex and Claude maps may differ in raw field names but must produce the event types in Task 2. Include `parent_agent_id` when supplied. `PostToolUse` for a write-capable tool emits `mutation_observed`; a semantic Decision is still only emitted by explicit Kernel input.

- [ ] **Step 5: Run conformance-focused hook tests.**

Run: `cargo test --test hook_flow`

Expected: PASS.

- [ ] **Step 6: Commit Task 5.**

```bash
git add src/hook.rs src/cli.rs src/lib.rs tests/hook_flow.rs
git commit -m "[FEAT](hooks): normalize native lifecycle capture"
```

## Task 6: Build project-local adapter planning, installation, verification, and uninstall

**Files:**

- Create: `src/adapter.rs`
- Modify: `src/commands.rs`
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Create: `tests/adapter_install.rs`

**Consumes:** CLI types, host matrix, and canonical event names.

**Produces:** idempotent `adapter plan`, `install`, `verify`, and `uninstall` commands.

```rust
pub struct AdapterPlan {
    pub host: AdapterHost,
    pub config_path: PathBuf,
    pub bindings: Vec<HookBinding>,
    pub capabilities: CaptureCapabilities,
    pub capture_grade: CaptureGrade,
}

pub struct HookBinding {
    pub binding_id: String,
    pub event: String,
    pub matcher: Option<String>,
    pub command: String,
}

pub fn plan_adapter(source: &Path, host: AdapterHost) -> Result<AdapterPlan, DevMapError>;
pub fn install_adapter(plan: AdapterPlan) -> Result<InstallReport, DevMapError>;
pub fn verify_adapter(source: &Path, host: AdapterHost) -> Result<VerifyReport, DevMapError>;
pub fn uninstall_adapter(source: &Path, host: AdapterHost) -> Result<InstallReport, DevMapError>;
```

- [ ] **Step 1: Write failing installer tests.**

Create source fixtures with existing unrelated Codex/Claude hooks. Assert `plan` makes no file changes. Assert `install` adds one DevMap binding per required event, preserves unrelated entries byte-for-byte where possible, is idempotent, and leaves Git HEAD, index, refs, branch, config, and worktree files unchanged except the explicitly named config file. Assert malformed existing JSON refuses without overwrite. Assert uninstall removes only bindings whose `binding_id` starts with `devmap/v1/`.

- [ ] **Step 2: Run the installer test and confirm failure.**

Run: `cargo test --test adapter_install`

Expected: FAIL because `adapter.rs` is absent.

- [ ] **Step 3: Implement Codex and Claude binding plans.**

Generate JSON configurations using the documented nested `hooks[event] -> matcher group -> hooks[]` structure. Codex bindings cover `SessionStart`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse`, `PreCompact`, `PostCompact`, `SubagentStart`, `SubagentStop`, `Stop`, and `SessionEnd`. Claude bindings cover the equivalent supported lifecycle events. Each command is exactly:

```text
devmap hook handle --host <host> --event <event> --binding-id devmap/v1/<host>/<event>
```

Use no shell interpolation and no project-relative script path. Add a concise `statusMessage` where that host supports it.

- [ ] **Step 4: Implement safe JSON merge.**

Parse JSON into `serde_json::Value`; require a top-level object and object-valued `hooks` field if present. Append only a missing DevMap binding ID. Write through `<config>.devmap-tmp`, `sync_all`, then atomic rename. On Windows, remove only a DevMap-created backup after the new file is verified. Do not change a malformed or unrecognized existing config.

- [ ] **Step 5: Implement verify and uninstall reports.**

`verify` reports present/missing/modified binding IDs, Kernel command path, capabilities, and Capture Grade. A changed binding reports `capture_grade=D` with an explicit drift reason. `uninstall` refuses malformed config and removes empty DevMap-created matcher groups only when they contain no non-DevMap hooks.

- [ ] **Step 6: Run installer tests.**

Run: `cargo test --test adapter_install`

Expected: PASS.

- [ ] **Step 7: Commit Task 6.**

```bash
git add src/adapter.rs src/commands.rs src/cli.rs src/lib.rs src/error.rs tests/adapter_install.rs
git commit -m "[FEAT](adapters): install native project hooks safely"
```

## Task 7: Implement the Generic MCP stdio adapter

**Files:**

- Create: `src/mcp.rs`
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Create: `tests/mcp_stdio.rs`
- Modify: `tests/adapter_install.rs`

**Consumes:** Capture Kernel, Journal Store, Event Envelope, and CLI `McpArgs`.

**Produces:** `devmap mcp --source PATH` JSON-RPC server plus Generic MCP install descriptor.

```rust
pub fn serve_mcp(
    source: &Path,
    reader: impl std::io::BufRead,
    writer: impl std::io::Write,
) -> Result<(), DevMapError>;

pub const MCP_TOOLS: [&str; 4] = [
    "devmap_context",
    "devmap_record_requirement",
    "devmap_record_decision",
    "devmap_record_evidence",
];
```

- [ ] **Step 1: Write failing stdio protocol tests.**

Feed newline-delimited JSON-RPC requests through an in-memory reader. Test `initialize`, `tools/list`, `tools/call` for all four tools, invalid parameters, unknown method, notification without response, and multiple messages in a single stream. Assert stdout contains only one-line UTF-8 JSON-RPC messages and diagnostic output goes only to stderr.

- [ ] **Step 2: Run the MCP test and confirm failure.**

Run: `cargo test --test mcp_stdio`

Expected: FAIL because `serve_mcp` is absent.

- [ ] **Step 3: Implement minimal MCP stdio transport.**

Read one UTF-8 JSON-RPC object per newline and write one response per newline. Implement MCP initialization and `tools/list`; reject unsupported protocol versions with JSON-RPC errors. Do not add HTTP transport or a daemon. The implementation must never print banners, logging, or diagnostics on stdout.

- [ ] **Step 4: Implement tool handlers through the Capture Kernel.**

`devmap_context` returns the current workspace, branch, HEAD, journal location, and effective Capture Grade. The other tools require `session_id`, `agent_id`, and their typed inputs from Task 4; they append validated events and return the record SHA-256. Tool calls cannot create source Git mutations in Phase 1B.

- [ ] **Step 5: Add Generic MCP descriptor planning.**

For `adapter plan --host generic-mcp`, render a JSON descriptor to `.devmap/mcp.json` containing only the command array `["devmap", "mcp", "--source", "."]` and `transport="stdio"`. Installer behavior follows Task 6's preview, atomic write, verify, and uninstall rules.

- [ ] **Step 6: Run Generic MCP and installer tests.**

Run: `cargo test --test mcp_stdio --test adapter_install`

Expected: PASS.

- [ ] **Step 7: Commit Task 7.**

```bash
git add src/mcp.rs src/cli.rs src/lib.rs tests/mcp_stdio.rs tests/adapter_install.rs
git commit -m "[FEAT](mcp): expose generic capture tools"
```

## Task 8: Add cross-host conformance fixtures and Phase 1B acceptance test

**Files:**

- Create: `tests/adapter_conformance.rs`
- Create: `tests/phase_1b_acceptance.rs`
- Modify: `tests/support/mod.rs`
- Modify: `README.md`
- Modify: `README.zh-CN.md`

**Consumes:** all Phase 1B public interfaces.

**Produces:** repeatable proof that equivalent Codex, Claude, and Generic flows produce equivalent canonical evidence and that Phase 1A behavior remains intact.

- [ ] **Step 1: Write a shared scenario fixture.**

Create one semantic scenario: session start, human request, subagent start, explicit autonomous Decision, write mutation, test Evidence, compaction, and session stop. Express expected canonical event types, actor parent relation, route, sequence, and Capture Grade in a test helper shared by all three host inputs.

- [ ] **Step 2: Write failing conformance and acceptance tests.**

Assert that Codex and Claude fixture inputs produce equal canonical event bytes after removing host-only metadata; Generic MCP calls produce the same semantic sequence. Assert no raw prompt text is written. Assert a write mutation without explicit semantic trace produces exactly one `capture_gap`. Assert adapter installation changes only `.codex/hooks.json`, `.claude/settings.json`, or `.devmap/mcp.json`; Git snapshot checks prove no branch, index, ref, commit, or configuration mutation.

- [ ] **Step 3: Run tests and confirm failure before any required acceptance patch.**

Run: `cargo test --test adapter_conformance --test phase_1b_acceptance`

Expected: FAIL until all Phase 1B behavior is integrated.

- [ ] **Step 4: Fix only exposed integration defects.**

Do not add workflow orchestration, branch creation, source commits, remote calls, or PR behavior in this task.

- [ ] **Step 5: Document operator flow.**

Add a short English and Chinese guide showing `adapter plan`, review, install, host trust/review, `adapter verify`, capture grade interpretation, Generic MCP descriptor use, local journal location, and uninstall. State clearly that Phase 1B does not manage branches, worktrees, commits, or pushes.

- [ ] **Step 6: Run complete Phase 1B verification.**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
target/release/devmap adapter --help
target/release/devmap hook --help
target/release/devmap mcp --help
git diff --check
```

Expected: all commands exit `0`; release help exposes Phase 1B commands.

- [ ] **Step 7: Commit Task 8.**

```bash
git add tests/adapter_conformance.rs tests/phase_1b_acceptance.rs tests/support/mod.rs README.md README.zh-CN.md
git commit -m "[TEST](capture): verify native adapter conformance"
```

## Final review gate

- [ ] Re-read every Phase 1B requirement in the approved spec and map it to a passing focused test or an explicit later phase.
- [ ] Confirm the only source-root writes are explicitly selected adapter configuration files.
- [ ] Confirm Git snapshots prove no source branch, worktree, index, commit, ref, config, stash, or remote mutation occurred.
- [ ] Confirm malformed hook input and malformed existing config cannot overwrite files or fabricate events.
- [ ] Confirm `adapter verify` reports actual Grade rather than a requested Grade.
- [ ] Confirm `work/` was neither changed nor staged.
- [ ] Run the full formatting, lint, test, build, and help suite from a clean checkout.
- [ ] Request code review before integrating Phase 1B.
