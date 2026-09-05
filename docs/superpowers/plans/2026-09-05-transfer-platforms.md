# Transfer platforms

Approved direction: commit stations, worktree platforms, one passenger per linked task, and future routes ending in a planned arrival area rather than a fictional merge commit. Preserve the warm-light metro appearance and existing task navigation.

Implementation: make workspace facts collapsible, surface passenger counts and current common-ancestor navigation, mark actual fork/merge stations, and project bounded route plans outside the actual topology. Creation origin remains unknown unless recorded; plan start is not worktree creation. No new MCP tool, Git write, merge queue or inferred passenger movement.

Verify with core geometry/intent tests, renderer interaction tests, resource budget and plugin/API regressions, then inspect desktop and narrow browser layouts. Do not install or merge this increment without a later user request.

## Verification

- 128 Node core/renderer tests passed, including future geometry isolation, passenger state separation, details focus, destination certainty, and plan selection refresh/removal.
- 16 Rust UI/plugin contract tests passed; development binary builds and `cargo fmt --check` passes.
- Browser checked at the default desktop size and 390px width. Platform contents remain readable; workspace details retain SUMMARY keyboard focus; planned destination navigation exposes the arrival zone.
- Read-only review findings resolved. Screenshots use an explicitly synthetic Git repository and two synthetic task passengers, not the user's active repository.
- Recorded worktree creation events and passenger movement history remain outside this increment. Common ancestor navigation is current Git evidence only.

## Passenger lifecycle refinement

User-approved refinement: one passenger is an existing unarchived chat at the exact worktree. Completed, waiting, idle and unloaded execution states do not remove a passenger. Archived/deleted records are historical; legacy existence is unknown. A complete fresh unarchived-chat inventory is required to certify unattended status. Unattended dirty/ahead work raises attention; clean included work only suggests review for cleanup. Mainline clean platforms may remain empty. No automatic cleanup or reversal.

Implemented separate lifecycle input/output, `task_observation.scope`, conservative omitted completeness, and `workspace_facts.passengers` for Agent context. Scope-less legacy snapshots cannot certify an empty platform. Model summaries are recomputed after output-budget truncation. Deleted records remain inspect-only. Platform, overview, roster, freshness updates, and plugin collection instructions follow the same definition.

Validation: 133 Node tests pass; model suite (30 tests) passed plus the new expiry regression and a post-truncation rerun; MCP (16), map/Agent (8), plugin (3), and UI contracts (13) pass after updating obsolete expectations. Development build, formatting and all-target Clippy pass. Browser fixtures verify a finished unarchived passenger remains present and an all-archived unmerged worktree raises unattended-work attention. Fixture screenshots are under `target/transfer-verification/passenger-*.png`.

Lifecycle refresh depends on host inventory synchronization. This increment does not watch private chat databases or infer deletion from a missing record. Installed plugin and main remain unchanged.
