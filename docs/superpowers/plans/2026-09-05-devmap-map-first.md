# DevMap Map-first Implementation Plan

> Execute inline using the approved map-first scope. Each deliverable has its own failing test, minimal implementation, and focused verification before full regression.

**Goal:** Ship one map workflow with three map tools, persistent route intent, truthful future routes, and unchanged source Git state.

**Architecture:** A bounded local route-plan journal feeds the existing Dock read model. Public map tools reuse the existing viewer and capture stack; old tool names remain callable compatibility aliases, while discovery advertises three map tools plus three existing capture tools.

**Tech Stack:** Existing Rust/Cargo, fs2, serde, embedded HTML/JS, Node test fixtures.

**Spec:** `docs/superpowers/specs/2026-09-05-devmap-map-first-and-public-interface-design.md`.

## Constraints

- Base: committed topology/navigation baseline `4d344d1`, isolated branch `codex/devmap-map-first`.
- No source Git writes through map APIs. No enforcement queue or rollback engine.
- Actual history solid; future intent dashed; unknown connections remain unknown.
- Preserve bounded input/output, task ownership checks, native STDIO and authenticated browser fallback.
- Plan writes are explicitly metadata writes, with revision checks, stable route identity and retry identity.

## Deliverables

- [x] Route storage (`src/route_plan.rs`, `tests/route_plan.rs`): create/update/reopen a plan, retain immutable start, reject stale revisions, deduplicate identical request IDs, reject unsafe paths and oversize inputs. Run `cargo test --test route_plan` before and after implementation. Use real temporary repos and compare source snapshots.
- [x] Public surface (`src/mcp.rs`, `tests/map_mcp.rs`): discover six public tools, call open/read/set, reject invalid arguments, read plan detail, preserve legacy aliases. Initial failing requests: `devmap_set_route_plan` with a valid temporary worktree and `expected_revision: 0`; `devmap_read_map` after reopening runtime.
- [x] Read model (`src/dock.rs`): include bounded plans and diagnostics; refresh revision changes when metadata changes, no invented integration result; link plans to exact worktree identity.
- [x] Map UI (`assets/dock.html`, `assets/metro-core.js`, Node tests): optional plan validation, dashed future segment and hollow destination, selected plan details, no dashes on actual branch history. Malformed plan input must fail before rendering; snapshots without plans still work.
- [x] Single Skill and plugin config (`plugins/devmap`): use new read/open tools, browser surface argument only when required, plan write instructions and default write policy; preserve record tools and old-call compatibility.
- [x] Verification: `cargo fmt --check`, full `cargo test`, `cargo clippy --all-targets -- -D warnings`, Node renderer/metro tests, and bounded browser inspection at narrow and wide sizes. Document commands, results, and limits in README.

No automatic merge, plugin installation, publishing, or worktree cleanup belongs to this change.

## Verification — 2026-09-05

- `cargo test --quiet`: full regression passed, exit 0.
- `cargo clippy --all-targets -- -D warnings`: passed, exit 0.
- `cargo fmt --check`: passed.
- `node --test tests/metro_core.cjs tests/dock_renderer.cjs`: 122 passed, 0 failed.
- Temporary real Git repository: created a feature worktree and route through MCP, reopened/read the stored route, and inspected the live browser map at the default wide size and 390 × 844. Solid history, dashed future stops, target label and narrow-screen scrolling rendered correctly. Browser automation timed out when clicking the plan title, so native-browser details-click validation remains unconfirmed; renderer coverage passed.
- Independent code review found deleted-worktree edits, conflict response content, and unborn HEAD handling. Each finding received a regression test and a fix. Storage tests also cover concurrent updates, corrupt journals, retry identity and unchanged source state.

Logs are local build artifacts under `target/map-verification/`.

## Explicit limits

- Future routes currently live inside worktree cards; a continuous projected return line onto the trunk remains future UI work.
- History-change warnings compare consecutive snapshots within one live service. They are not a persistent classification of cherry-picks, reverts, or human intent. A normal descendant revert remains actual history.
- Missing or invalid plan metadata produces diagnostics; the map does not reconstruct intent or restore Git state.
- The isolated branch contains this development implementation. No plugin installation, commit, merge, or publication was performed.

## Agent delivery increment — 2026-09-05

Implemented the approved extension to the existing route flow:

- `delivery` records manual/auto-merge intent, up to 12 completion conditions, and a bounded authorization source. Auto-merge intent requires a target and nonempty conditions/source. Missing delivery in old journals or new writes defaults to manual; updates are full replacements, and manual mode revokes recorded auto-merge intent.
- `devmap_read_map` with `view: agent` returns the current worktree (or an exact worktree `entity_id`), its observed facts, associated plans, map revision, warnings and truncation. Unknown worktrees fail rather than silently switching context. All associated plans remain visible; the Agent must select the task's active route.
- Execution fields explicitly mark checks and authorization unverified and do not certify merge readiness. Metadata is not authenticated user permission. Actual Git execution remains outside DevMap; the Skill instructs executing Agents to verify existing user authorization, conditions, fresh source/target state and human changes.
- Map cards and route details show the same delivery agreement. Legacy snapshots still render. A route's planned target may differ from the observed integration target in workspace facts.

Validation: focused storage/API tests, 123 Node tests, 13 UI contract tests, formatting and warning-free Clippy passed. Independent read-only review reported no concrete bugs. Real temporary Git preview displayed delivery conditions and unverified status; keyboard activation successfully opened route details and their authorization source. Screenshot: `target/map-verification/delivery-map.png`.

Final regression results:

- `cargo test --quiet -- --skip bounded_large_reduction_and_live_revision_meet_mvp_latency_targets`: 268 passed, exit 0; only the timing benchmark excluded.
- Unskipped debug regression failed the existing 1-second reduction benchmark (1.095 seconds); focused debug samples ranged from 0.918 to 1.279 seconds. A concurrent full release run also failed that threshold (2.193 seconds). Do not describe either full run as passing.
- After other tests finished, `cargo test --release --test live_dock_acceptance bounded_large_reduction_and_live_revision_meet_mvp_latency_targets -- --nocapture` passed: reduction 765.986 ms, Git change to visible revision 1.752 seconds. Timing is sensitive to local load; no acceptance threshold was relaxed.
- Removed diagnostic instrumentation and experimental serialization changes. Retained only the empty-plan short circuit to avoid needless serialization during size checks. Canonical hashing and output budgets are unchanged.
- Final logs: `delivery-debug-functional.log`, `delivery-release-isolated.log`, `delivery-ui.log`, and `delivery-clippy-final.log` under `target/map-verification/`.

## Main integration verification — 2026-09-05

The user requested merging the development code into main and updating the local installation. Pre-merge full-suite runs reproduced the reduction timing failure, including serial execution. The reducer now overlaps independent read-only topology and relationship scans instead of serializing their Git process latency. Current-worktree validation runs before either scan; a new empty-inventory regression confirms an error is returned instead of a panic. Existing live-service topology caching is preserved.

The complete release suite passed with `--test-threads=1` (`merge-final-suite.log`), followed by the additional empty-inventory regression and warning-free Clippy. The focused acceptance run measured 503.868 ms for 100 worktrees / 1000 records and 1.914 seconds from Git change to visible revision. Node tests: 123 passed. Acceptance thresholds were not changed. The original working directory's uncommitted files are preserved; local main uses a separate worktree.
