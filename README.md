# DevMap

DevMap builds a Git-backed evidence map of software development so that a human or AI can determine:

1. Why was this route taken?
2. Was the route required by a human or selected autonomously by an Agent?
3. Did the Agent have authority to make that decision?
4. Which alternatives were rejected?
5. Which evidence demonstrates that the result works?
6. Has the decision been superseded?

Phase 1A establishes the trustworthy starting point for that map. It records an explicitly reviewed Common Ground and an immutable Adoption Boundary. It deliberately does not infer decisions, alternatives, or rationale from history before adoption.

## What Phase 1A provides

- A single Rust binary with no Node.js, daemon, database, or account requirement.
- Read-only inspection of an existing source Git repository.
- An independent, ordinary Git Context Repository.
- A reviewable Common Ground draft on bootstrap/initial.
- Explicit human approval before promotion to Context main.
- Canonical JSON, SHA-256 content IDs, and immutable Common Ground and Approval objects.
- Independent status verification with non-zero exit on integrity failure.
- Capture Grade C: explicit CLI capture is available; automatic Agent hooks are not active.

Phase 1A does not provide Agent hooks, autonomous Decision capture, route branches, PR Capsules, a merge gate, signed attestations, a Graph Database, or the topology Viewer. Those are subsequent phases.

## Build and test

Rust 1.96.1 or newer is required.

~~~text
cargo build
cargo test --all-targets
cargo clippy --all-targets --all-features -- -D warnings
~~~

The release binary is produced at target/release/devmap on Unix-like systems and target/release/devmap.exe on Windows.

## Local walkthrough

Use separate directories for the source repository and Context Repository. The Context path must not be inside the source repository.

~~~powershell
cargo build
$devmap = Resolve-Path .\target\debug\devmap.exe
& $devmap init --source C:\work\payment-service --context C:\work\payment-service-context --goal "Adopt DevMap from the current main commit" --requirement "docs/requirements.md#payment-safety"
& $devmap common-ground approve --context C:\work\payment-service-context --actor "Dylan"
& $devmap status --context C:\work\payment-service-context
& $devmap status --context C:\work\payment-service-context --json
~~~

The init command prints the source commit used as the Adoption Boundary, whether local source changes existed, the draft hash, and the exact approval command. Review bootstrap/common-ground-draft.json in the Context Repository before approving it.

## Storage and ownership

The source repository remains untouched. DevMap executes only an allowlist of read-only Git commands there.

The independent Context Repository uses normal branches and commits:

~~~text
payment-service-context/
  .devmap-context.json
  objects/
    approval/<sha256>.json
    common-ground/<sha256>.json
  manifests/common-ground.json
  state/current.json
~~~

Only DevMap-owned paths are staged. If an unexpected file is present when DevMap attempts a commit, the commit is refused. Repository-local commit identity is DevMap Bot with devmap-bot@localhost. No custom refs or Git Notes are created.

The draft initially exists only on bootstrap/initial. Approval creates the canonical objects and manifest, commits them, fast-forwards main, and removes the bootstrap branch. Re-running init after approval is rejected because later changes must use an explicit supersession workflow rather than overwrite Common Ground.

## Requirement Trace versus Agent Decision

A quoted human requirement is stored as Requirement Trace. Following that instruction does not create an Agent Decision. A later capture phase will create Agent Decisions only when an Agent autonomously selects a meaningful route among alternatives and has the authority to do so.

## Integrity and recovery

devmap status recomputes canonical object hashes and validates content IDs, manifest references, Approval binding, the Adoption Boundary, and the absence of forbidden DevMap or Notes refs. A dirty Context working tree is reported. Missing or modified evidence produces an invalid report and a non-zero process exit.

Because Context main is an ordinary Git branch, backup, cloning, access control, and recovery use standard Git operations. Restore damaged working files from a trusted commit or clone, then run devmap status again. Do not repair canonical objects by editing them in place.

## Planned phases

1. Canonical Capture Kernel, host adapters, capability handshake, SessionStart and SubagentStart propagation, capture gaps, and adapter benchmarks.
2. Route branches, autonomous Decision and Claim schemas, PR Context Capsules, merge gate, Context Bot ingestion, supersession, and signed in-toto evidence.
3. A read-only local Viewer that projects W3C PROV relationships into a shared force-directed topology with semantic zoom and PM evidence-chain inspection.

The complete product requirements are in docs/ai-development-map-requirements.md. The Phase 1A implementation plan is in docs/superpowers/plans/2026-08-26-devmap-phase-1a-core.md.

