# Git Workflow Orchestrator and Native Adapter Design

**Date:** 2026-08-27
**Status:** Approved design; implementation planning pending
**Scope:** DevMap Phase 1B and Phase 1C

## 1. Purpose

DevMap will help people who use web-based coding agents but do not know Git
workflows such as branches, worktrees, checkpoints, commits, and pull
requests. It will do this without weakening the evidence-chain model:

- the agent may choose a Git route only when its authority allows it;
- every autonomous, material route choice is an Agent Decision;
- explicit human direction remains a Requirement Trace, not an Agent Decision;
- the source repository, local Git state, and shared Context graph never claim
  more certainty than the observed evidence supports.

The product has three user-facing gears:

| Gear | Source Git actions |
| --- | --- |
| `manual` | Observe, record, and propose only. |
| `controlled` | Automatically create branches/worktrees and commits after all gates pass. Push, PR, merge, and cleanup need additional authority. |
| `full-auto` | Automatically perform authorized push, PR, and cleanup actions. Merge remains separately delegated and gated. |

`controlled` is the default. `full-auto` is an explicit human opt-in, bounded
by organization and repository policy. Neither mode can disable safety gates.

## 2. Architectural decision

Use one host-neutral Git Workflow Orchestrator. Do not duplicate Git policy
inside Codex, Claude, or generic adapters.

```text
Codex / Claude / Generic Adapter
                |
       Canonical Event Protocol
          +-----+-----+
          |           |
          v           v
 Capture Kernel   Mutation Ledger
          |           |
          +-----+-----+
                v
     Git Workflow Orchestrator
       +--------+--------+
       |        |        |
       v        v        v
 Policy Engine Safety Gate Git Executor
                |
                v
        Source Git Repository
                |
       remote confirmation
                v
     Context Bundle -> Context Bot
```

Component responsibilities are deliberately non-overlapping:

- **Host Adapter:** installs and receives host lifecycle events; identifies
  host, agent, subagent, session, and capabilities. It has no Git policy.
- **Canonical Event Protocol:** is the only adapter-to-core interface.
- **Capture Kernel:** classifies Requirement Trace, Agent Decision, Activity,
  Evidence, and Capture Gap. It does not execute Git actions.
- **Mutation Ledger:** append-only local observation of mutations, causality,
  file state, and action state.
- **Git Workflow Orchestrator:** creates structured Git Action Proposals.
- **Policy Engine:** resolves authority and gear.
- **Safety Gate:** accepts or rejects a proposed action from observed state.
- **Git Executor:** executes only accepted actions; it does not decide them.
- **Context Bridge:** prepares pending Context Bundles. A Context Bot remains
  the only writer of shared Context branches.

The core never imports host-specific behavior. Adapters depend only on the
protocol and the `devmap hook handle <event>` entry point.

## 3. Evidence and authority rules

The following classification is mandatory:

| Cause of action | Required record |
| --- | --- |
| A human explicitly asks for a branch, commit, or route | Requirement Trace plus Activity |
| A deterministic project policy triggers an action | Policy Reference plus Activity |
| An agent chooses a material route, checkpoint, or isolation strategy | Agent Decision plus Activity |
| A lifecycle rule causes a checkpoint | Activity; no invented Agent Decision |

An Agent Decision requires rationale, alternatives considered, authority,
scope, and a revisit trigger. An action outside delegated authority remains
`proposed` and cannot be executed.

The system must not infer historical motivations from code, diffs, old commits,
or old chat text. It must also not invent a Decision merely because a mutation
occurred. A mutation without sufficient trace creates `capture.gap`.

## 4. Route, branch, and worktree policy

Every mutation-capable task receives a stable `route_id` before its first
write. Human-provided branch names are labels, not durable route identity.

```text
Task starts
  |
  +-- read-only -> no branch or worktree action
  |
  +-- human specified a route -> safety-check then obey
  |
  +-- compatible existing route -> reuse it
  |
  +-- otherwise evaluate isolation and host capability
          |
          +-- simple, clean, single writer -> branch in current worktree
          +-- concurrent, long/risky, dirty, conflicting -> branch + worktree
```

Create an isolated worktree when any of the following is true:

- another writing session is active in the repository;
- a subagent will modify code independently;
- the current branch belongs to another route;
- the current worktree contains changes of unknown ownership;
- the task has independent stages or a high-risk refactor;
- a human, project policy, or authorized agent requests strong isolation.

Branch-in-place is allowed only when the source worktree is clean, on the
default branch, before any mutation, has one writer, and the host cannot safely
rebind to a newly created worktree or the task is short and single-route.

Adapters declare `workspace_rebind` in their capability handshake. If it is
supported, the session moves to the new worktree. If it is not supported,
DevMap may branch in the clean current worktree. If neither is safe, DevMap
blocks the first mutation and presents a recovery command; it must not pretend
that isolation happened.

Other invariants:

- Do not automatically stash, rebase a dirty branch, switch an already-mutated
  worktree, or delete a branch/worktree in `controlled` mode.
- `full-auto` cleanup needs remote merge confirmation, a clean worktree, and no
  unique unpushed commit.
- Autonomous isolation choices are recorded as Agent Decisions. Human route
  instructions are Requirement Traces.

## 5. Commit policy

Commits reflect explainable development nodes, not arbitrary time intervals.

### Commit classes

- **Milestone Commit:** one coherent intent has completed and has appropriate
  verification.
- **Checkpoint Commit:** saves a recoverable state before compaction, handoff,
  session end, or a high-risk next stage. It can be incomplete but is explicitly
  marked as such.
- **Human-directed Commit:** follows an explicit human request after safety
  checks.

The Orchestrator may propose a commit when a requirement or subgoal completes,
an autonomous route produces evidence, a coherent repair/refactor/test/document
unit is ready, a handoff or compaction is imminent, or a risky phase is about
to begin. Configured age and size thresholds may cause reevaluation but cannot
split a semantically incomplete unit on their own.

Before committing, the Safety Gate verifies:

- the index was clean at session start;
- every staged path is in an attributed manifest;
- no unknown files, conflicts, overlapping human edits, or secrets are present;
- the commit contains one coherent intent;
- relevant configured checks have run;
- required Requirement/Decision, Activity, and Ledger records exist; and
- branch HEAD has not changed since proposal creation.

The Executor must never use `git add .` or `git add -A`. It stages only an
explicit manifest. The first release does not auto-split individual hunks; a
mixed-origin or mixed-intent file pauses automation.

Milestone Commit validation normally passes. A deliberate red TDD test or
incomplete recovery state may be a Checkpoint Commit only when marked
`expected_failure` or `incomplete`; it is never presented as verified evidence.

Commit messages remain human-readable and use only lightweight
`DevMap-Route` and `DevMap-Activity` trailers. The full evidence graph is bound
in the Context Bundle after the commit hash is known.

## 6. Dynamic gear changes

Gear is a dynamic authority lease, not a separate workflow. A user can change
gear in the same session and route without restarting, recloning, or discarding
the ledger.

```text
manual <--------> controlled <--------> full-auto
        immediate downshift       human-authorized upgrade
```

On a gear change DevMap:

1. waits for an in-flight atomic Git action to reach a safe boundary;
2. snapshots and reconciles HEAD, index, worktree, and ledger state;
3. invalidates queued action proposals and recomputes them;
4. calculates the new effective authority;
5. records `authority.changed`; and
6. continues the existing session and route.

Effective authority is the minimum of organization policy, repository policy,
the current human gear selection, route authority, and per-action authority.
Agents may recommend but never self-upgrade a gear. Downgrades cancel all
unstarted automatic work immediately. On manual-to-automatic transition,
attributable changes can be adopted; unknown changes require reconciliation.

The simple web-coding UI labels are: **I control**, **Automatically organize
code**, and **Automatically deliver**. It always previews the next proposed
action and its reason. The advanced UI can reveal Git mechanics.

## 7. Canonical event protocol and adapters

All adapters emit the same ordered Event Envelope:

```json
{
  "schema_version": "1",
  "event_id": "evt_...",
  "event_type": "mutation.observed",
  "sequence": 42,
  "occurred_at": "...",
  "host": {"name": "codex", "adapter_version": "..."},
  "actor": {"agent_id": "...", "parent_agent_id": "..."},
  "context": {
    "session_id": "...",
    "route_id": "...",
    "repository": "...",
    "worktree": "...",
    "branch": "...",
    "head": "..."
  },
  "payload": {}
}
```

First-release event types are:

- `session.started`, `session.stopped`;
- `instruction.observed`;
- `agent.started`, `agent.stopped`;
- `tool.requested`, `tool.completed`;
- `mutation.observed`;
- `decision.recorded`, `evidence.recorded`;
- `context.compacting`, `context.compacted`;
- `git.action.proposed`, `git.action.authorized`, `git.action.executed`,
  `git.action.failed`;
- `authority.changed`; and
- `capture.gap`.

The capability handshake declares lifecycle coverage, pre-mutation blocking,
subagent support, workspace rebind, tool-result observation, commit mapping,
and known gaps. It determines Capture Grade.

Codex and Claude adapters use their native lifecycle Hooks. Generic hosts use
DevMap MCP tools and optional wrappers; their Capture Grade is based on actual
capability, never on a prompt alone. Project-local installation is the default:

```text
devmap adapter plan --host codex
devmap adapter install --host codex
devmap adapter install --host claude
devmap adapter install --host generic-mcp
devmap adapter verify
```

The installer preview is mandatory, merges idempotently, preserves unrelated
hooks, and tags DevMap-owned handlers for safe uninstall. It adds project-local
Codex and Claude hook entries rather than copying Kernel rules into settings.
CI checks Kernel and policy hash drift.

Local journal entries are append-only and deduplicated by `event_id` and
per-session sequence. Canonical context stores structured traces, necessary
quotations, and hashes by default—not full, unbounded transcripts. Raw
transcript storage is an opt-in restricted evidence feature with a retention
policy.

## 8. Local state, recovery, and remote consistency

Transient state is kept under the resolved Git directory for each worktree:

```text
<worktree git dir>/devmap/
  sessions/       session state
  ledger/         append-only mutation and causality records
  actions/        Git Action state
  pending/        Context Bundles awaiting publication
  locks/          short-lived coordination locks
```

It is not a custom ref and is not committed to the source repository. A local
code commit is a recovery anchor but is not shared canonical context.

Each action advances through:

```text
planned -> authorized -> safety_checked -> executing -> observed
                                           |              |
                                           +--> failed     +--> succeeded
                                                          +--> reconciliation_required
```

Before execution DevMap saves an `action_id`, expected branch/HEAD/upstream,
index and worktree fingerprints, allowed manifest, authority result, and
expected postcondition. It marks success only after observing the postcondition.

On restart:

- matching postcondition means `succeeded`;
- unchanged state is safely retryable;
- partially changed or externally changed state requires reconciliation;
- uncertain push queries the remote SHA;
- uncertain PR creation queries provider identity and idempotency data;
- failed cleanup preserves an orphaned worktree rather than deleting it.

DevMap must never use `reset --hard`, automatic stash, or destructive rollback.
Locks coordinate writers but do not prove file ownership. Expired locks are
recovered only after the action state is inspected.

The publication boundary is explicit:

```text
local commit -> pending Context Bundle -> remote push confirmation
             -> Context Bot validation -> Context route branch update
```

If code is remotely published but the bundle is missing, DevMap creates a
`context_gap`; high-risk merge gates must block until evidence is reconciled.

Manual mode works with any Capture Grade. Controlled automation requires
mutation-to-commit association. Full automation requires complete native capture
or a specific, time-limited human waiver. Loss of capability automatically
downgrades the active gear.

## 9. Verification and phased delivery

Verification has five layers:

| Layer | Required proof |
| --- | --- |
| Schema and Kernel | Correctly distinguish human direction, policy, decisions, activities, and evidence. |
| Protocol conformance | A shared event scenario yields equivalent canonical output across adapters. |
| Hook integration | Install, lifecycle, subagent inheritance, compaction, and drift tests on supported hosts. |
| Git sandbox | Branch/worktree/commit/push behavior in real disposable repositories. |
| End-to-end | Human instruction through Context Bundle and remote-confirmed route update. |

Required adversarial scenarios include: dirty unknown worktrees, overlapping
human and agent edits, generated files and lockfiles, agent crashes during every
action phase, uncertain pushes, gear transitions, capture degradation, and
missing bundles. Tests must also assert no `git add .`, `git add -A`, automatic
stash, or `reset --hard` invocation.

Run equivalent representative coding sessions on Codex, Claude, and Generic MCP
to measure event coverage, Decision false positives, recovery success, Hook
latency, and context overhead.

Delivery order:

1. **Phase 1B:** Canonical Event Protocol, Capture Kernel integration,
   Mutation Ledger, project-local installer, Codex/Claude/Generic adapters, and
   conformance tests. It makes no source Git mutation.
2. **Phase 1C.1:** controlled branch/worktree/commit automation, recovery, and
   dynamic gear changes in local Git sandboxes.
3. **Phase 1C.2:** full-auto push and remote confirmation.
4. **Phase 1C.3:** provider adapters for GitHub, GitLab, and other PR systems.
   Until a provider adapter is installed, full-auto stops after authorized Git
   push rather than claiming PR or merge support.

## 10. Non-goals and invariants

This design does not add historical decision reconstruction, automatic business
requirement validation, invisible WIP commits, custom refs, automatic merge or
rebase without separate authority, or a single privileged shared write path for
all agents.

At every stage, the system must be able to answer:

1. Why did it take this route?
2. Was it human-specified, policy-required, or agent-chosen?
3. Did the agent have authority?
4. Which alternatives were not taken?
5. What evidence validates the outcome?
6. Has this decision been superseded?
