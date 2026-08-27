# DevMap Phase 1C.1 Controlled Git Workflow Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Deliver local, controlled Git automation that can safely create a route branch, isolate work in a worktree when possible, make attributable semantic commits, recover from interruption, and switch between manual and controlled gears without rewriting history.

**Architecture:** Phase 1C.1 extends the Phase 1B journal into a local Route Registry, Mutation Ledger, Action Journal, Policy Engine, Safety Gate, and Git Executor. The Orchestrator proposes actions from observed state; the executor performs only accepted local actions. Existing native hooks request evaluation at safe lifecycle points. Remote push, PR, merge, and deletion remain deferred.

**Tech Stack:** Rust 1.96, Cargo, clap 4, serde, serde_json, sha2, time, thiserror, tempfile, Phase 1B modules, and the system Git executable. No daemon, database, Git host API, custom refs, Git Notes, automatic stash, automatic rebase, or destructive rollback.

**Spec:** `docs/superpowers/specs/2026-08-27-git-workflow-orchestrator-design.md`

## Global Constraints

- Complete and integrate the approved Phase 1B plan before starting this plan.
- Default mode is `controlled`; `manual` and `full-auto` are valid authority states.
- Phase 1C.1 executes only local `create_branch`, `create_worktree`, and `commit` actions.
- `full-auto` state may be recorded, but push, PR, merge, and deletion must return `remote_capability_missing` until Phase 1C.2/1C.3.
- The executor must never use `git add .`, `git add -A`, `git stash`, `git reset --hard`, automatic rebase, force push, or branch/worktree deletion.
- A source action is executed only after authority, safety, ownership, and expected-HEAD checks succeed.
- An unknown or overlapping modification must downgrade automation to reconciliation; it must not be committed.
- Human directions are Requirement Traces; deterministic policy firings are Activities; material autonomous route choices are Agent Decisions.
- Every Git action is durably recorded as `planned -> authorized -> safety_checked -> executing -> observed -> succeeded|failed|reconciliation_required`.
- A source commit remains local pending Context Bundle publication; Phase 1C.1 must not claim it is team-visible.
- `work/` is user-owned and must never be modified, staged, or committed.
- Every behavior change begins with a failing automated test.

---

## Target file layout

```text
src/
  action.rs              # Action states, expected postconditions, durable records
  adapter.rs             # Extend hook bindings only after workflow core exists
  capture.rs             # Extend mutation record with ownership references
  cli.rs                 # workflow subcommands
  error.rs
  git.rs                 # Strict read/write source Git wrappers and snapshots
  hook.rs                # Safe lifecycle integration points
  journal.rs             # Phase 1B event journal
  lib.rs
  ownership.rs           # File/path provenance and explicit commit manifest
  policy.rs              # Gear, authority leases, and deterministic action rules
  route.rs               # Route Registry and branch/worktree planner
  workflow.rs            # Orchestrator, Safety Gate, and executor facade
tests/
  action_recovery.rs
  controlled_workflow.rs
  ownership_gate.rs
  policy_modes.rs
  route_planner.rs
  workflow_hook_flow.rs
  phase_1c1_acceptance.rs
```

## Task 1: Add workflow CLI, modes, and policy-domain objects

**Files:**

- Create: `src/policy.rs`
- Modify: `src/cli.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Create: `tests/policy_modes.rs`

**Consumes:** Phase 1B adapter and capture types.

**Produces:** `WorkflowMode`, `AuthorityLease`, `WorkflowPolicy`, and CLI entry points used by all later workflow tasks.

```rust
pub enum WorkflowMode { Manual, Controlled, FullAuto }

pub struct AuthorityLease {
    pub mode: WorkflowMode,
    pub scope: LeaseScope,
    pub granted_by: String,
    pub granted_at: String,
    pub expires_at: Option<String>,
    pub revoked_at: Option<String>,
}

pub struct WorkflowPolicy {
    pub repository_ceiling: WorkflowMode,
    pub require_clean_index: bool,
    pub required_checks: Vec<String>,
    pub permit_checkpoint_commits: bool,
}

pub enum WorkflowCommand {
    Status(WorkflowStatusArgs),
    Mode(WorkflowModeArgs),
    Evaluate(WorkflowEvaluateArgs),
    Reconcile(WorkflowReconcileArgs),
}
```

- [ ] **Step 1: Write failing mode and authority tests.**

Test the ordering `manual < controlled < full_auto`, policy ceiling clamping, expiry, explicit revocation, and that an agent identity cannot grant an upgrade. Test that a human `AuthorityLease` produces `authority_changed`, while a policy-driven action does not create an Agent Decision.

- [ ] **Step 2: Run the focused test and confirm failure.**

Run: `cargo test --test policy_modes`

Expected: FAIL because workflow types do not exist.

- [ ] **Step 3: Implement typed modes and configuration loading.**

Read optional repository policy from `.devmap/workflow.json`. If absent, use `controlled` ceiling and require a clean index. Do not create the policy file during status or evaluation. `workflow mode --set <mode>` writes a session lease under the local DevMap Git directory and appends `authority_changed` through the Phase 1B journal.

- [ ] **Step 4: Implement the CLI surface.**

Expose:

```text
devmap workflow status --source PATH --session-id ID
devmap workflow mode --source PATH --session-id ID --set manual|controlled|full-auto --actor HUMAN
devmap workflow evaluate --source PATH --session-id ID --agent-id ID
devmap workflow reconcile --source PATH --session-id ID
```

Reject blank human actors and reject `--set full-auto` when policy ceiling is lower. Do not perform Git writes in any Task 1 command.

- [ ] **Step 5: Run focused tests.**

Run: `cargo test --test policy_modes`

Expected: PASS.

- [ ] **Step 6: Commit Task 1.**

```bash
git add src/policy.rs src/cli.rs src/lib.rs src/error.rs tests/policy_modes.rs
git commit -m "[FEAT](workflow): add authority gear policies"
```

## Task 2: Implement route registry and deterministic branch/worktree planner

**Files:**

- Create: `src/route.rs`
- Modify: `src/git.rs`
- Modify: `src/lib.rs`
- Create: `tests/route_planner.rs`
- Modify: `tests/support/mod.rs`

**Consumes:** `WorkflowPolicy`, Phase 1B `SourceWorkspace`, capture session context.

**Produces:** `RouteState`, `RouteRegistry`, and `RoutePlan` used by the Orchestrator.

```rust
pub struct RouteState {
    pub route_id: String,
    pub session_id: String,
    pub branch: String,
    pub worktree: PathBuf,
    pub base_head: String,
    pub status: RouteStatus,
}

pub enum RouteAction { Reuse, CreateBranchInPlace, CreateWorktree, ReconciliationRequired }

pub struct RoutePlan {
    pub action: RouteAction,
    pub route: RouteState,
    pub rationale: Vec<String>,
    pub autonomous_decision_required: bool,
}

pub fn plan_route(input: RoutePlanningInput, registry: &RouteRegistry) -> RoutePlan;
```

- [ ] **Step 1: Extend fixtures for concurrent sessions and worktrees.**

Add helpers that create a second linked worktree and a source repository snapshot consisting of HEAD, branch, porcelain status, staged diff, refs, and local config. Add a helper to assert an exact snapshot is unchanged.

- [ ] **Step 2: Write failing route planner tests.**

Cover: read-only task yields no route action; explicit human branch yields `Reuse`/branch action with no autonomous Decision; a clean default branch and single writer yields `CreateBranchInPlace`; a second active writer, unknown dirty state, independent subagent, or high-risk flag yields `CreateWorktree`; a dirty current route belonging to another session yields reconciliation. Assert `route_id` is derived from repository fingerprint plus session ID and does not change when a branch label changes.

- [ ] **Step 3: Run the test and confirm failure.**

Run: `cargo test --test route_planner`

Expected: FAIL because the registry and planner are absent.

- [ ] **Step 4: Implement shared route registry.**

Extend `SourceWorkspace` with `common_git_dir` from `git rev-parse --git-common-dir`. Store route registry records under `<common-git-dir>/devmap/routes/<route-id>.json`, using canonical JSON and atomic replace. This is local state, not a custom ref. Keep separate per-worktree Session journals from Phase 1B.

- [ ] **Step 5: Implement branch naming and worktree target selection.**

Use `devmap/<route-id>-<slug>` branch names, where the slug is sanitized and non-authoritative. Compute worktree target as a sibling under the source parent’s `.devmap-worktrees/<repository-name>/<route-id>`; reject a path escaping that root. The planner creates a worktree plan only; it does not execute Git commands.

- [ ] **Step 6: Record autonomous route rationale.**

For a planner choice not directly dictated by a human trace or a deterministic one-condition policy, require an `AgentDecisionInput` before authorizing the RoutePlan. A planner output without required decision is `ReconciliationRequired`.

- [ ] **Step 7: Run tests.**

Run: `cargo test --test route_planner`

Expected: PASS.

- [ ] **Step 8: Commit Task 2.**

```bash
git add src/route.rs src/git.rs src/lib.rs tests/route_planner.rs tests/support/mod.rs
git commit -m "[FEAT](workflow): plan isolated development routes"
```

## Task 3: Build mutation ownership ledger and explicit commit manifests

**Files:**

- Create: `src/ownership.rs`
- Modify: `src/capture.rs`
- Modify: `src/git.rs`
- Modify: `src/lib.rs`
- Create: `tests/ownership_gate.rs`

**Consumes:** Phase 1B mutation events, route state, and workspace snapshots.

**Produces:** `MutationLedger`, `OwnershipState`, and `CommitManifest` used by the Safety Gate.

```rust
pub enum OwnershipState { SessionOwned, Derived, PreexistingUnknown, Overlapping, Unknown }

pub struct PathMutation {
    pub path: String,
    pub before_sha256: Option<String>,
    pub after_sha256: Option<String>,
    pub actor_id: String,
    pub cause_event_id: String,
    pub ownership: OwnershipState,
}

pub struct CommitManifest {
    pub route_id: String,
    pub paths: Vec<String>,
    pub activity_ids: Vec<String>,
    pub verification_status: VerificationStatus,
}

pub fn build_manifest(...) -> Result<CommitManifest, DevMapError>;
```

- [ ] **Step 1: Write failing ownership tests.**

Test a clean session modifying one path through a recorded tool event becomes `SessionOwned`. Test files dirty before session start become `PreexistingUnknown`; a later unobserved path becomes `Unknown`; simultaneous edits to an observed path by different actors become `Overlapping`; a lockfile changed by a recorded package-install command becomes `Derived`. Assert manifest construction rejects every state except `SessionOwned` and validated `Derived`.

- [ ] **Step 2: Run the focused tests and confirm failure.**

Run: `cargo test --test ownership_gate`

Expected: FAIL because ownership types are absent.

- [ ] **Step 3: Implement read-only mutation snapshots.**

Add explicit Git read commands for `status --porcelain=v1 -z`, `diff --name-status`, `diff --cached --name-status`, and `hash-object -- <path>` only for already-observed paths. Capture baseline index and worktree status before a session’s first mutation; never scan arbitrary file contents.

- [ ] **Step 4: Implement conservative ownership resolution.**

Use exact observed path and event causality, not guessed line authorship. The first version does not attempt hunk splitting. A changed path appearing before baseline or without a matching post-tool mutation event cannot enter a manifest. Generated paths require an allowlisted command event and a nonblank input-cause event ID.

- [ ] **Step 5: Implement manifest integrity and journal linkage.**

Sort and deduplicate paths, reject empty manifests, bind the manifest to route, current HEAD, activities, and verification status, and write `commit_manifest_prepared` as a local action/event. Do not stage paths in Task 3.

- [ ] **Step 6: Run tests.**

Run: `cargo test --test ownership_gate`

Expected: PASS.

- [ ] **Step 7: Commit Task 3.**

```bash
git add src/ownership.rs src/capture.rs src/git.rs src/lib.rs tests/ownership_gate.rs
git commit -m "[FEAT](workflow): gate commits by mutation ownership"
```

## Task 4: Add durable action records, locks, and crash reconciliation

**Files:**

- Create: `src/action.rs`
- Modify: `src/error.rs`
- Modify: `src/lib.rs`
- Create: `tests/action_recovery.rs`

**Consumes:** Route plans, commit manifests, workflow modes, and local Git directories.

**Produces:** `GitAction`, `ActionRecord`, `ActionStore`, and `RepositoryLease`.

```rust
pub enum GitActionKind { CreateBranch, CreateWorktree, Commit }
pub enum ActionState {
    Planned, Authorized, SafetyChecked, Executing, Observed,
    Succeeded, Failed, ReconciliationRequired,
}

pub struct ActionRecord {
    pub action_id: String,
    pub kind: GitActionKind,
    pub state: ActionState,
    pub expected_head: String,
    pub expected_postcondition: serde_json::Value,
    pub error: Option<String>,
}

pub struct RepositoryLease { pub path: PathBuf, pub holder: String, pub expires_at: String }
```

- [ ] **Step 1: Write failing action-state tests.**

Test every legal transition and reject `Planned -> Executing`, `Succeeded -> Executing`, and missing expected postconditions. Test an interrupted `Executing` action: matching observed state becomes `Succeeded`, untouched state becomes retryable `SafetyChecked`, and divergent state becomes `ReconciliationRequired`. Test a live lease blocks a second writer; an expired lease is inspectable but not deleted before action reconciliation.

- [ ] **Step 2: Run tests and confirm failure.**

Run: `cargo test --test action_recovery`

Expected: FAIL because action types do not exist.

- [ ] **Step 3: Implement durable action files.**

Store one canonical JSON action record at `<common-git-dir>/devmap/actions/<action-id>.json`. Write updates using a temporary sibling, `sync_all`, and rename. Preserve transition history in an append-only `history` array. `action_id` is `sha256(route_id + kind + expected_head + monotonic sequence)` with a `action:` prefix.

- [ ] **Step 4: Implement lease acquisition without extra dependencies.**

Use `OpenOptions::create_new(true)` for `<common-git-dir>/devmap/locks/<repository-fingerprint>.lock`. Include holder, acquired time, expiry, and action ID in canonical JSON. On expired lock, read the referenced action; only then atomically rename the old lock to a forensic `.expired` name before creating a new lease.

- [ ] **Step 5: Run recovery tests.**

Run: `cargo test --test action_recovery`

Expected: PASS.

- [ ] **Step 6: Commit Task 4.**

```bash
git add src/action.rs src/error.rs src/lib.rs tests/action_recovery.rs
git commit -m "[FEAT](workflow): persist recoverable git actions"
```

## Task 5: Implement Safety Gate and strict local Git Executor

**Files:**

- Create: `src/workflow.rs`
- Modify: `src/git.rs`
- Modify: `src/lib.rs`
- Modify: `src/error.rs`
- Create: `tests/controlled_workflow.rs`

**Consumes:** policy, route plan, ownership manifest, action records, and leases.

**Produces:** `WorkflowOrchestrator::evaluate`, `SafetyReport`, and local action execution.

```rust
pub struct SafetyReport {
    pub accepted: bool,
    pub reasons: Vec<String>,
    pub observed_head: String,
}

pub struct WorkflowOrchestrator;
impl WorkflowOrchestrator {
    pub fn evaluate(&self, input: WorkflowInput) -> Result<Vec<ActionRecord>, DevMapError>;
    pub fn execute(&self, action_id: &str) -> Result<ActionRecord, DevMapError>;
    pub fn reconcile(&self, action_id: &str) -> Result<ActionRecord, DevMapError>;
}
```

- [ ] **Step 1: Write failing controlled-flow tests.**

Cover: clean main/single writer creates `devmap/<route-id>-slug` in place; second writer creates a separate worktree; dirty unknown worktree produces no Git command; a human specified branch is honored after safety checks; unsupported rebind produces a planned worktree plus `requires_rebind` status without pretending the current agent moved; `manual` produces proposals only; `controlled` executes local accepted actions; `full-auto` returns `remote_capability_missing` for a push request.

- [ ] **Step 2: Run the tests and confirm failure.**

Run: `cargo test --test controlled_workflow`

Expected: FAIL because the Orchestrator is absent.

- [ ] **Step 3: Implement Safety Gate evaluation.**

Before every execution, re-read branch, HEAD, porcelain status, staged status, active route registry, ownership manifest, and policy. Reject changed HEAD, dirty initial index, missing required Agent Decision, unknown/overlapping paths, missing checks, invalid mode, stale/absent lease, or a worktree path outside DevMap’s configured root.

- [ ] **Step 4: Implement explicit Git command arrays.**

Use only these source Git writes in Phase 1C.1:

```text
git switch -c <branch> <expected-head>
git worktree add <absolute-path> -b <branch> <expected-head>
git add -- <manifest-path-1> <manifest-path-2> ...
git commit -m <message> --trailer DevMap-Route:<route-id> --trailer DevMap-Activity:<activity-id>
git restore --staged -- <manifest paths>    # only recovery after precondition index was clean
```

Do not call an unlisted Git write command. Build command arguments as `OsString` values; never create a shell command string.

- [ ] **Step 5: Implement action execution and observation.**

Acquire lease, transition action records, execute one command, release lease only after recording observation. A branch action succeeds only when branch and HEAD match expected state. A worktree action succeeds only when `git worktree list --porcelain` contains the exact root and branch. A commit succeeds only when new HEAD differs, manifest paths are clean, and the commit message contains the expected trailers. If a commit fails after staging, restore only explicit manifest paths when the index still matches the saved pre-stage fingerprint; otherwise leave the scene and require reconciliation.

- [ ] **Step 6: Run controlled workflow tests.**

Run: `cargo test --test controlled_workflow`

Expected: PASS.

- [ ] **Step 7: Commit Task 5.**

```bash
git add src/workflow.rs src/git.rs src/lib.rs src/error.rs tests/controlled_workflow.rs
git commit -m "[FEAT](workflow): execute controlled local git actions"
```

## Task 6: Wire lifecycle evaluation and seamless gear switching

**Files:**

- Modify: `src/hook.rs`
- Modify: `src/adapter.rs`
- Modify: `src/workflow.rs`
- Modify: `src/cli.rs`
- Create: `tests/workflow_hook_flow.rs`

**Consumes:** Phase 1B hooks and all Phase 1C.1 workflow interfaces.

**Produces:** safe automatic evaluation at SessionStart, PreToolUse, PostToolUse, PreCompact, SubagentStop, and Stop boundaries.

- [ ] **Step 1: Write failing lifecycle tests.**

Test that SessionStart in controlled mode plans a route but does not mutate a dirty source. Test that PreToolUse checks route readiness before a write-capable tool. Test PostToolUse records mutation before proposing a commit. Test PreCompact/Stop proposes a checkpoint only when a complete explicit manifest exists. Test `workflow mode --set manual` cancels all unstarted action records, while a running action reaches its observed state first. Test manual-to-controlled re-evaluates state and requires reconciliation for unknown changes.

- [ ] **Step 2: Run tests and confirm failure.**

Run: `cargo test --test workflow_hook_flow`

Expected: FAIL until hook integration invokes the Orchestrator.

- [ ] **Step 3: Add hook-to-workflow boundary.**

After Phase 1B event normalization succeeds, call `WorkflowOrchestrator::evaluate` only for the lifecycle events above and only when the active Capture Grade satisfies controlled-mode requirements. Record every proposal and safety result as canonical local event/action data. A hook timeout or workflow error produces `capture_gap`/`workflow_unavailable` and leaves source Git unchanged.

- [ ] **Step 4: Preserve adapter-grade safety.**

If the adapter’s capability verification no longer supports mutation-to-commit association, automatically set effective mode to `manual` and append `authority_changed` with `reason=capture_grade_downgrade`. Do not modify the human-requested lease; only the effective execution mode is clamped.

- [ ] **Step 5: Run lifecycle and regression tests.**

Run: `cargo test --test workflow_hook_flow --test hook_flow --test adapter_conformance`

Expected: PASS.

- [ ] **Step 6: Commit Task 6.**

```bash
git add src/hook.rs src/adapter.rs src/workflow.rs src/cli.rs tests/workflow_hook_flow.rs
git commit -m "[FEAT](workflow): automate safe lifecycle checkpoints"
```

## Task 7: Add Phase 1C.1 acceptance coverage and operator documentation

**Files:**

- Create: `tests/phase_1c1_acceptance.rs`
- Modify: `README.md`
- Modify: `README.zh-CN.md`
- Modify: `docs/ai-development-map-requirements.md`

**Consumes:** all Phase 1C.1 modules.

**Produces:** end-to-end proof and user documentation for controlled automation.

- [ ] **Step 1: Write the failing end-to-end acceptance scenario.**

Create a source repository with an approved Phase 1A Context Repository and an installed Phase 1B adapter. Start a controlled Session, record a human requirement and an agent route decision, create a route, simulate one attributed source edit plus passing evidence, trigger a checkpoint, and assert a local branch commit with DevMap trailers. Then add an unknown human edit and assert the next automatic commit is blocked. Switch to manual and assert no later source Git write occurs. Crash an action record before observation, restart reconciliation, and assert no duplicate commit.

- [ ] **Step 2: Run acceptance test and confirm failure before integration fixes.**

Run: `cargo test --test phase_1c1_acceptance`

Expected: FAIL until all controlled-flow pieces are connected.

- [ ] **Step 3: Fix only acceptance defects.**

Do not implement remote push, PR creation, merge, force-push handling, deletion, provider APIs, Graph DB, or a web UI.

- [ ] **Step 4: Update bilingual operator documentation.**

Document the three gears, how to switch mode, what actions controlled mode may execute, worktree rebind limitations, why unknown changes stop automation, local journal and action locations, recovery/reconcile commands, and explicit statement that the commit is local until later Context publication. Document `full-auto` as deferred for remote actions in Phase 1C.2/1C.3.

- [ ] **Step 5: Run complete verification.**

Run:

```bash
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --all-targets --all-features
cargo build --release
target/release/devmap workflow --help
target/release/devmap workflow status --help
git diff --check
```

Expected: all commands exit `0`; no test relies on an external Git host.

- [ ] **Step 6: Commit Task 7.**

```bash
git add tests/phase_1c1_acceptance.rs README.md README.zh-CN.md docs/ai-development-map-requirements.md
git commit -m "[TEST](workflow): validate controlled git automation"
```

## Final review gate

- [ ] Map every Phase 1C.1 specification requirement to a passing test or explicitly deferred Phase 1C.2/1C.3 work.
- [ ] Confirm every source Git write is one of the Task 5 allowlisted argument arrays.
- [ ] Confirm tests assert no use of broad staging, stash, hard reset, automatic rebase, remote push, branch deletion, or worktree deletion.
- [ ] Confirm a stale or divergent action enters reconciliation and never repeats a commit.
- [ ] Confirm mode downgrade cancels unstarted actions and no Agent self-upgrade is possible.
- [ ] Confirm unknown or overlapping changes block commits.
- [ ] Confirm source commit output is labelled local/pending rather than team-published.
- [ ] Confirm `work/` was neither changed nor staged.
- [ ] Run full format, lint, test, build, and help checks from a clean checkout.
- [ ] Request code review before integrating Phase 1C.1.
