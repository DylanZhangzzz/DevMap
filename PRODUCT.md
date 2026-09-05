# DevMap

<!-- impeccable:product-schema 1 -->

## Platform

web

## Users

Developers supervising multiple local Codex tasks and Git worktrees from a narrow right-side Browser Dock. They need to identify who is working where and notice unfinished code before leaving a workspace or releasing.

## Product Purpose

Show repository relationships and current work together: branches and commits, checked-out workspaces, their associated tasks/Agents, and evidence-backed integration state.

Success means a user can trace the origin and destination of work, see the exact task title under its workspace, navigate to that task when the host supports it, and identify work that needs human attention.

## Operating Context

- Rust application with a self-contained HTML/CSS/JavaScript Dock embedded in the binary.
- Local Git facts and host-supplied task metadata are separate sources with separate freshness.
- Browser Dock and embedded MCP surfaces must communicate through their supported host capabilities.
- The user approved the third metro visual concept and subsequently corrected its missing branch connections.

## Capabilities and Constraints

- Branches are movable references on a commit DAG; their drawn lanes are a presentation.
- Workspace means a Git worktree identified by its canonical path and stable worktree ID.
- One workspace can have multiple Codex tasks; a task's displayed Agent is not an independently navigable entity unless the host actually supplies such an identity.
- A workspace attaches to its checked-out HEAD, including detached HEAD; a common ancestor is not proof of historical branch creation.
- Task association requires exact canonical workspace identity. A matching title, branch name or project name is insufficient.
- Commits, local modifications, integration, upstream publication, and task execution are different facts.
- Unknown or stale observations must remain visibly uncertain. Missing telemetry does not establish that no Agent is working.
- The Dock is read-only with respect to source Git state. Task navigation must not execute Git mutations.
- Existing model and HTML byte limits, portable transport and injection defenses are retained unless an explicit compatibility change is documented.

## Brand Commitments

DevMap uses a London Underground-inspired schematic and branch palette, with light surfaces, workspace stations and visible Agent rows. Branch color identifies a branch; warning symbols and state text independently identify risk. The topology must remain traceable when distances and ordinary commits are compressed.

## Evidence on Hand

- `assets/dock.html`, `src/dock.rs`, `src/git_relationship.rs`: current implementation.
- `docs/audits/2026-09-04-devmap-impeccable-audit.md`: measured pre-development findings.
- `docs/superpowers/specs/2026-09-04-devmap-metro-topology-design.md`: accepted direction and explicit corrections.
- The original third concept is archived at `docs/audits/assets/2026-09-04-metro-preflight/approved-concept-3.png`. It is a visual reference, not proof of correct Git relationships or runtime behavior.

## Product Principles

1. Preserve the connections that let people explain where work came from and where it went.
2. Keep workspaces primary in the operational hierarchy and make their tasks recognizable.
3. Prioritize unfinished work without classifying ordinary active development as abandonment.
4. Keep evidence, inference and unavailable observations distinguishable.
5. Judge completion in the actual sidebar and installed runtime, as well as in tests.
