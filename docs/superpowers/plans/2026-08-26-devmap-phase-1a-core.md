# DevMap Phase 1A Core and Common Ground Implementation Plan

> **For agentic workers:** Execute this plan task by task with test-driven development. Do not broaden Phase 1A without an approved requirement change.

**Goal:** Deliver a Rust command-line foundation that establishes an immutable Common Ground for an existing Git project, stores canonical evidence in an independent Context Repository, and reports a verifiable capture status.

**Architecture:** A single Rust binary reads the source repository without modifying it and writes only to an explicitly selected Context Repository. Canonical objects use deterministic JSON and content-derived identifiers. Initialization creates a reviewable draft on an ordinary bootstrap branch; explicit approval promotes immutable objects to Context main. Status independently verifies hashes and repository invariants.

**Tech stack:** Rust 1.96.1, Cargo, clap 4, serde, serde_json, sha2, time, thiserror, tempfile, and the system Git executable.

**Authoritative specification:** docs/ai-development-map-requirements.md version 0.3.

## Phase 1A scope and invariants

- Do not reconstruct or claim to explain decisions made before adoption.
- Do not use custom Git refs or Git Notes.
- Never commit, push, tag, or change configuration in the source repository.
- Use one independent Context Repository for each source repository.
- Use ordinary main and bootstrap/initial branches in this phase.
- DevMap may write Git commits only in the explicitly selected Context Repository.
- Canonical evidence is compact, deterministic JSON with SHA-256 content identity.
- Common Ground requires explicit human approval before it becomes canonical.
- Phase 1A reports Capture Grade C because it provides explicit CLI capture but no automatic hooks.
- Do not add a daemon, Graph Database, network service, Node.js runtime, route branches, capture hooks, adapters, attestations, or topology Viewer in this phase.
- Every behavior change begins with a failing automated test.

## Target file layout

~~~
Cargo.toml
README.md
src/
  canonical.rs
  cli.rs
  commands.rs
  context.rs
  domain.rs
  error.rs
  git.rs
  lib.rs
  main.rs
tests/
  canonical.rs
  cli_help.rs
  common_ground_flow.rs
  context_repo.rs
  domain.rs
  git_inspector.rs
  phase_1a_acceptance.rs
  status_flow.rs
  support/mod.rs
~~~

## Task 1: Establish the Rust CLI foundation

**Files**

- Create: Cargo.toml
- Create: src/lib.rs
- Create: src/main.rs
- Create: src/cli.rs
- Create: src/error.rs
- Create: tests/cli_help.rs

**Public interfaces**

~~~rust
pub fn run<I, T>(args: I) -> Result<CommandOutput, DevMapError>
where
    I: IntoIterator<Item = T>,
    T: Into<std::ffi::OsString> + Clone;

pub struct CommandOutput {
    pub stdout: String,
}
~~~

**Steps**

- [ ] Write a failing CLI test asserting that help lists init, common-ground approve, and status.
- [ ] Run cargo test --test cli_help and confirm the expected failure.
- [ ] Add the Cargo package, dependency declarations, parser types, shared error type, and a thin main that prints successful output and maps errors to a non-zero exit.
- [ ] Keep command handlers unimplemented only through typed UnsupportedCommand errors; do not panic.
- [ ] Run cargo test --test cli_help and cargo check.
- [ ] Commit only Task 1 files with message: feat: establish devmap cli foundation

## Task 2: Implement canonical JSON and content identities

**Files**

- Create: src/canonical.rs
- Modify: src/lib.rs
- Create: tests/canonical.rs

**Public interfaces**

~~~rust
pub fn canonical_json<T: serde::Serialize>(value: &T) -> Result<Vec<u8>, DevMapError>;
pub fn sha256_hex(bytes: &[u8]) -> String;
pub fn content_id(kind: &str, bytes: &[u8]) -> String;
~~~

**Required behavior**

- Object keys are recursively sorted.
- No insignificant whitespace is emitted.
- UTF-8 text is preserved.
- Arrays retain semantic order.
- Floating point values are rejected from canonical domain objects.
- A content ID has the form kind:sha256-HEX.

**Steps**

- [ ] Write failing tests for stable key order, nested objects, Unicode, array order, and a known SHA-256 vector.
- [ ] Run cargo test --test canonical and confirm failure.
- [ ] Implement canonical serialization through a recursively normalized serde_json Value.
- [ ] Reject non-finite or otherwise unsupported numeric representations with a typed error.
- [ ] Run cargo test --test canonical twice to prove deterministic output.
- [ ] Commit only Task 2 files with message: feat: add canonical evidence encoding

## Task 3: Define Common Ground domain objects and truth invariants

**Files**

- Create: src/domain.rs
- Modify: src/lib.rs
- Create: tests/domain.rs

**Domain objects**

~~~rust
pub struct SourceAnchor {
    pub repository_fingerprint: String,
    pub remote_url: Option<String>,
    pub head_commit: String,
    pub default_branch: Option<String>,
    pub dirty_at_adoption: bool,
}

pub struct RequirementTrace {
    pub source_path: Option<String>,
    pub anchor: Option<String>,
    pub quoted_requirement: String,
}

pub struct CommonGroundDraft {
    pub schema_version: String,
    pub created_at: String,
    pub source: SourceAnchor,
    pub goal: String,
    pub requirements: Vec<RequirementTrace>,
    pub historical_scope: HistoricalScope,
}

pub enum HistoricalScope {
    NotReconstructed,
}

pub struct ApprovalEvent {
    pub actor: String,
    pub approved_at: String,
    pub draft_sha256: String,
}

pub struct CommonGround {
    pub schema_version: String,
    pub adopted_at: String,
    pub adoption_boundary_commit: String,
    pub source: SourceAnchor,
    pub goal: String,
    pub requirements: Vec<RequirementTrace>,
    pub historical_scope: HistoricalScope,
    pub approval_id: String,
}
~~~

**Required behavior**

- Goal and approval actor must be non-empty after trimming.
- The adoption boundary equals the observed source HEAD.
- historical_scope can only state not_reconstructed in Phase 1A.
- Requirement Trace represents what a human requested; it is not an Agent Decision.
- Approval references the exact draft hash.

**Steps**

- [ ] Write failing tests for valid construction and each rejected invariant.
- [ ] Run cargo test --test domain and confirm failure.
- [ ] Implement constructors that enforce invariants rather than exposing unchecked construction.
- [ ] Add serialization names that remain stable across Rust refactors.
- [ ] Run cargo test --test domain.
- [ ] Commit only Task 3 files with message: feat: model adoption common ground

## Task 4: Add a read-only source Git inspector

**Files**

- Create: src/git.rs
- Modify: src/lib.rs
- Create: tests/support/mod.rs
- Create: tests/git_inspector.rs

**Public interfaces**

~~~rust
pub struct SourceGitInspector {
    root: std::path::PathBuf,
}

impl SourceGitInspector {
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, DevMapError>;
    pub fn inspect(&self) -> Result<SourceAnchor, DevMapError>;
}
~~~

**Allowed source-repository Git operations**

- git rev-parse --show-toplevel
- git rev-parse HEAD
- git symbolic-ref --short -q HEAD
- git remote get-url origin
- git status --porcelain=v1

**Required behavior**

- Reject a non-repository and an unborn source repository.
- Fingerprint the normalized remote URL when present; otherwise fingerprint the absolute repository root plus HEAD.
- Record whether the working tree was dirty without copying file contents.
- Preserve source HEAD, branch, index, worktree, refs, and Git configuration byte-for-byte where observable.

**Steps**

- [ ] Build temporary Git repository fixtures in tests/support/mod.rs.
- [ ] Write a failing test that snapshots HEAD, status, refs, and local config before and after inspect.
- [ ] Run cargo test --test git_inspector and confirm failure.
- [ ] Implement the allowlisted read-only command runner with argument arrays, never shell strings.
- [ ] Add clear errors for missing Git, non-repository paths, and unborn HEAD.
- [ ] Run cargo test --test git_inspector.
- [ ] Commit only Task 4 files with message: feat: inspect source repositories read only

## Task 5: Create the independent Context Repository store

**Files**

- Create: src/context.rs
- Modify: src/lib.rs
- Create: tests/context_repo.rs

**Public interfaces**

~~~rust
pub struct ContextRepo {
    root: std::path::PathBuf,
}

impl ContextRepo {
    pub fn create(path: impl AsRef<std::path::Path>) -> Result<Self, DevMapError>;
    pub fn open(path: impl AsRef<std::path::Path>) -> Result<Self, DevMapError>;
    pub fn write_canonical<T: serde::Serialize>(
        &self,
        kind: &str,
        value: &T,
    ) -> Result<StoredObject, DevMapError>;
    pub fn commit_all(&self, message: &str) -> Result<String, DevMapError>;
}
~~~

**Context Repository layout**

~~~
objects/
  common-ground/
  approvals/
manifests/
bootstrap/
  common-ground-draft.json
state/
  current.json
~~~

**Required behavior**

- Create an ordinary main branch with a repository metadata commit.
- Configure repository-local Bot identity only in the Context Repository.
- Use ordinary bootstrap/initial for the draft workflow.
- Reject a path that resolves inside the source repository when source and context paths are checked together.
- Store canonical objects under content-addressed filenames.
- Never create refs/devmap or refs/notes.
- Refuse to commit if unrelated unexpected files exist in the Context Repository.

**Steps**

- [ ] Write failing tests for initial branches, Bot-local config, object paths, and absence of custom refs.
- [ ] Run cargo test --test context_repo and confirm failure.
- [ ] Implement ContextRepo with a strict Git write wrapper scoped to its root.
- [ ] Stage explicit DevMap-owned paths rather than git add dot.
- [ ] Run cargo test --test context_repo.
- [ ] Commit only Task 5 files with message: feat: add context repository store

## Task 6: Implement Common Ground draft initialization

**Files**

- Create: src/commands.rs
- Modify: src/cli.rs
- Modify: src/lib.rs
- Create: tests/common_ground_flow.rs

**Command**

~~~
devmap init --source PATH --context PATH --goal TEXT
            [--requirement PATH[#ANCHOR]]
~~~

**Required behavior**

- Inspect current source HEAD and dirty state.
- Read only explicitly supplied requirement documents.
- If an anchor is supplied, require a uniquely matching heading or line marker.
- Store the exact selected requirement text and its source location.
- Create the independent Context Repository if absent.
- Create bootstrap/initial and write bootstrap/common-ground-draft.json.
- Commit the draft on bootstrap/initial.
- Do not create a canonical Common Ground object or merge to main.
- Print the draft hash, source boundary commit, dirty-state warning, and exact approval command.
- Re-running with identical input is idempotent; conflicting input is rejected.

**Steps**

- [ ] Write an end-to-end failing test from source fixture through draft commit.
- [ ] Add a failing test proving that unrelated source documents are not scanned.
- [ ] Add a failing test proving source HEAD and status do not change.
- [ ] Run cargo test --test common_ground_flow and confirm failure.
- [ ] Implement deterministic requirement extraction, path boundary checks, draft creation, and idempotency.
- [ ] Run cargo test --test common_ground_flow.
- [ ] Commit only Task 6 files with message: feat: initialize reviewable common ground

## Task 7: Implement explicit approval and canonical promotion

**Files**

- Modify: src/commands.rs
- Modify: src/cli.rs
- Modify: src/context.rs
- Modify: tests/common_ground_flow.rs

**Command**

~~~
devmap common-ground approve --context PATH --actor TEXT
~~~

**Required behavior**

- Require an existing draft on bootstrap/initial.
- Refuse approval if the working tree, draft hash, or branch state differs from the committed draft.
- Create a canonical ApprovalEvent bound to the draft hash.
- Create a canonical CommonGround bound to the observed adoption commit and approval ID.
- Write a manifest containing object IDs and schema version.
- Commit promotion on bootstrap/initial, fast-forward main, then delete bootstrap/initial.
- Keep canonical objects immutable; a different Common Ground requires a future superseding event, not overwrite.
- Print IDs, Context commit, and Capture Grade C.

**Steps**

- [ ] Write failing tests for successful approval, tampered draft, wrong branch state, and blank actor.
- [ ] Run the focused approval tests and confirm failure.
- [ ] Implement approval transaction with validation before any write.
- [ ] Add rollback-safe ordering: write, validate, commit, fast-forward, delete bootstrap branch.
- [ ] Run cargo test --test common_ground_flow.
- [ ] Commit only Task 7 files with message: feat: approve immutable common ground

## Task 8: Implement status and integrity verification

**Files**

- Modify: src/commands.rs
- Modify: src/cli.rs
- Modify: src/context.rs
- Create: tests/status_flow.rs

**Commands**

~~~
devmap status --context PATH
devmap status --context PATH --json
~~~

**Status output**

- Common Ground lifecycle: absent, draft, or approved.
- Adoption boundary commit.
- Context main commit.
- Capture Grade C and explanation that automatic hooks are not active.
- Integrity state: valid or invalid.
- Counts by canonical object kind.
- Dirty Context Repository warning.

**Required behavior**

- Recompute every manifest-referenced object hash.
- Reject missing, malformed, duplicated, or hash-mismatched objects.
- Verify the ApprovalEvent draft hash and CommonGround approval reference.
- Verify that no custom DevMap or Notes refs exist.
- Human-readable and JSON modes must derive from the same typed report.
- Exit non-zero on invalid integrity while still printing the report.

**Steps**

- [ ] Write failing tests for draft, approved, tampered-object, missing-object, and unexpected-ref states.
- [ ] Run cargo test --test status_flow and confirm failure.
- [ ] Implement a typed StatusReport and independent verifier.
- [ ] Serialize JSON output canonically enough for stable snapshots.
- [ ] Run cargo test --test status_flow.
- [ ] Commit only Task 8 files with message: feat: verify devmap context integrity

## Task 9: Complete the Phase 1A acceptance path and operator guide

**Files**

- Create: tests/phase_1a_acceptance.rs
- Create: README.md
- Modify any Phase 1A source file only when an acceptance test exposes a defect.

**Acceptance scenario**

1. Create an existing source repository with multiple historical commits and an uncommitted file.
2. Run init with a goal and one anchored requirement.
3. Verify no historical decision claims were generated.
4. Verify source HEAD, status, refs, and config are unchanged.
5. Inspect the draft on bootstrap/initial.
6. Approve it as a named human actor.
7. Verify main contains immutable Common Ground, ApprovalEvent, manifest, and state.
8. Verify bootstrap/initial is deleted and no custom refs exist.
9. Run status in text and JSON modes.
10. Tamper with an object and verify status fails with the object ID and expected hash.

**README content**

- Product boundary and the six questions the eventual system must answer.
- Phase 1A capabilities and explicit non-capabilities.
- Rust build and test commands.
- Local three-command walkthrough using two separate directories.
- Context Repository ownership and Bot write policy.
- Common Ground review responsibility.
- Capture Grade C explanation.
- Data layout and recovery notes.
- Next phases: capture kernel/adapters, route capsules/merge gate, then topology Viewer.

**Steps**

- [ ] Write the acceptance test first and confirm at least one unmet behavior.
- [ ] Fix only the exposed Phase 1A gaps.
- [ ] Run cargo fmt --check.
- [ ] Run cargo clippy --all-targets --all-features -- -D warnings.
- [ ] Run cargo test --all-targets --all-features.
- [ ] Build a release binary and run its help output.
- [ ] Follow the README walkthrough in fresh temporary repositories.
- [ ] Record exact command output in the implementation handoff; do not claim success from prior runs.
- [ ] Commit Task 9 files with message: docs: complete phase 1a acceptance guide

## Final review gate

- [ ] Compare every Phase 1A behavior against specification FR-001 through FR-083 and identify which requirements are delivered, deferred, or not applicable.
- [ ] Confirm that deferred requirements are not partially represented as working features.
- [ ] Confirm no files under work/ were changed or committed.
- [ ] Confirm the source repository fixture mutation checks pass.
- [ ] Confirm git for-each-ref shows no refs/devmap and no refs/notes in Context fixtures.
- [ ] Confirm all canonical object identifiers verify after a clean clone of the Context Repository.
- [ ] Run git diff --check.
- [ ] Run the complete formatting, lint, test, build, and smoke-test suite from a clean checkout.
- [ ] Request code review before integration.

## Deferred plans

The following require separate approved implementation plans after Phase 1A:

1. Canonical Capture Kernel, capability handshake, Agent/host adapters, SessionStart and SubagentStart propagation, capture_gap, and adapter benchmarks.
2. Route branches, Decision and Claim schemas, PR Context Capsules, merge gate, Context Bot ingestion, supersession, and signed in-toto evidence.
3. Read-only local Viewer server, W3C PROV projection, unified shared graph state, semantic zoom, force-directed topology, PM filters, and interactive evidence-chain inspection.

