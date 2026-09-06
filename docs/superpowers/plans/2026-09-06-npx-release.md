# DevMap npm release implementation plan

> For agentic workers: execute inline using executing-plans; independent review follows implementation.

**Goal:** deliver a no-Rust npm installation and automated multi-platform GitHub packaging.
**Architecture:** single npm bundle, Node launcher, native runner build matrix, tag release, opt-in trusted npm publication.
**Tech Stack:** Rust, Node >=22, npm, GitHub Actions.
**Spec:** ../specs/2026-09-06-npx-release-design.md

## Global constraints
Five native platforms; Git required; no install scripts; no runtime downloads; default local plugin preserved until npm publication; Cargo.toml is the version source; manual builds never publish.

## Task 1: package and launcher
- [x] Add behavior tests in packaging/tests/release.test.cjs; run with node --test packaging/tests/*.test.cjs and observe missing implementation.
- [x] Implement packaging/npm/cli.cjs and packaging/release.cjs. Export platform selection and package staging; validate version and required binaries before writing.
- [x] Build current executable with cargo build --release --locked.
- [x] Run packaging/smoke.cjs against a real staged npm tarball: CLI version, literal arguments/cwd with spaces, nonzero exit, MCP initialize and tools/list.

## Task 2: automation and documentation
- [x] Add .github/workflows/ci.yml and release.yml: native runners, Rust and launcher checks, aggregate asset validation, v-tag matching, read-only default permissions, opt-in OIDC publishing.
- [x] Document npx, tarball use before first publication, native downloads, Git/Node requirements, unsupported Linux musl, package scope setup and tag procedure.
- [x] Validate YAML, release script failure cases, native package payload and git diff.

## Task 3: competitive evidence
- [x] Read first-party Vibe Kanban, ATC Kanban and Codex Worktrees sources.
- [x] Compare delivered DevMap behavior to documented competitor capability; use not documented rather than absent.
- [x] Save dated findings, weaknesses, positioning hypothesis, and user validation thresholds under docs/research.
