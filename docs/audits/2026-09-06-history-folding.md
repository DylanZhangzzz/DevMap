# Continuous history folding

Scope: reversible summaries for long ordinary commit chains, based on main
`2000227e8bd03c9fa3dbd5f9d2d82e4862ea0800` in `codex/devmap-history-folding`.
No toolbar, workspace inspector, future-route styling or Git data changes.

- Four or more interior commits can collapse into endpoint hashes/subjects,
  a count and a three-dot rail break. Short chains remain unchanged.
- Forks, merges, HEADs, refs/tags, boundaries and journey evidence remain visible.
- Parent/child actions use original edges; navigation reveals hidden commits.
- Refresh and advancing HEAD preserve expansion and keyboard focus. Independent
  review reproduced the advancing-HEAD issue before its fix and confirmed both
  that fix and collapse persistence afterward.

Validation:

- `node --test tests/dock_renderer.cjs tests/metro_core.cjs`: 188 passed.
- `cargo test --quiet --test dock_ui_contract`: 13 passed. The running local
  preview initially locked the Windows executable; stopping that preview allowed
  the build and tests to complete without changing test behavior.
- Browser: 390 x 844 fixture, click dots to expand, Enter to collapse, retained
  focus, endpoint hashes and main label without overlap. Real repository checked
  at 390 x 844 and 1280 x 800: 66 ordinary commits folded into four summaries
  (9, 16, 14, 27), leaving 25 visible nodes.
- Browser screenshots were inspected inline. Later file capture returned
  `Unable to capture screenshot`; no saved screenshot artifact is claimed.
  Temporary viewport override reset; real preview retained for the right sidebar.
- Embedded asset: 185285 bytes; bounded source regression allowance increased
  from 176 to 184 KiB for this feature. The 512 KiB runtime cap is unchanged.
- `git diff --check`: passed. Validation completed on the isolated branch before
  publication; integration commits are recorded in Git history.
