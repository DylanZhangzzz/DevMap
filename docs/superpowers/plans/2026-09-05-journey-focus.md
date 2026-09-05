# Current journey focus

User-approved scope: work package A of the 2026-09-05 map review. Base: main c4bfee7. Implemented on codex/devmap-journey-focus; no merge, push or installed-plugin update in this increment.

## Behavior

- A persistent selected-workspace summary shows observed passengers, current HEAD, current common ancestor and each explicit destination. It remains available while inspecting commits or plans. Missing creation history is never inferred from the common ancestor or plan start.
- Platform destinations precede the roster. Zero activity categories and the duplicate roster summary are omitted; an expand-all action appears only when more than two passengers exist. Full titles, historical records and detailed workspace facts remain available.
- Focus journey temporarily compacts surrounding platforms and the selected platform's repeated detail, then fits its observed endpoints and future route. Show platform restores the full roster and readable workspace view. Long routes retain a readable workspace and endpoint links instead of shrinking into overview mode.
- A planned arrival is an accessible control. One observed source navigates directly; shared arrivals offer exact worktree choices. Refresh rederives membership by target, preserves focus, and never auto-selects a remaining source.
- Unknown, stale, unavailable and truncated observations stay qualified. Empty plan results with unavailable/truncated warnings cannot claim that no destination was recorded. Task inspection follows the task's observed worktree after relocation.
- Current journey state is separate from selected detail state. Switching workspaces, opening full map and removal of the selected workspace clear the appropriate compact presentation.

## Verification

- 142 Node core/renderer tests pass, including regression tests observed failing before implementing journey persistence, source-choice updates, focus cleanup, unavailable-plan wording and task relocation context.
- 13 Rust UI/resource tests and 3 plugin contract tests pass. Development binary builds; cargo fmt --check and git diff --check pass.
- Read-only review findings on selection, focus and partial data were addressed and re-reviewed.
- Browser checks at 1280×720 and 480×800. The same synthetic 3-commit / 2-worktree fixture used in the audit displays the common ancestor, current platform, planned arrival and complete dashed connection together at about 64% in journey focus; its original Full map view was about 23%. These are different views evaluated against the same journey-reading task, not a global zoom improvement claim.
- Narrow first view exposes the endpoint summary and destination before roster detail. Inspecting arrival preserves that summary. A real-repository Git-only preview renders unknown passenger coverage honestly. Synthetic chats were not opened as real tasks.
- Screenshots and test output are in this worktree's ignored target/verification directory: journey-desktop.png, journey-narrow.png, journey-arrival-narrow.png, journey-real.png and node-tests.txt.

## Bounds and stopping point

The self-contained UI regression budget increases from 144 to 148 KiB for persistent navigation; the existing 512 KiB transport cap is unchanged. No new dependency, MCP schema, Git write operation or history model is introduced.

Work package B (global overview layout), reliable creation-event capture, passenger-history replay and automated Git execution remain separate work. This increment is complete at the selected-journey level; it does not claim that a large repository's full overview has been redesigned.
