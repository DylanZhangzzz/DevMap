# Portrait sidebar adaptation

User goal: optimize DevMap for a normal portrait half-window sidebar while preserving the established map and passenger model.

## Changes

- Keep zoom, Full map and Locate current visible; collect pan, branches and workspace-list controls under More. Escape closes the disclosure and restores focus.
- At widths up to 760px, hide the empty intervention summary, shorten the journey passenger summary while retaining its observation qualifier and full title, and place Focus journey beside the heading.
- Keep journey endpoints on a horizontally scrollable row; preserve its scroll position on refresh.
- Give workspace cards more available width in a sidebar, reducing wrapped facts above conversations and subagents.
- Anchor the narrow More panel to the navigation region so wrapping cannot push it beyond the left edge.

## Verification

- Real local repository and observed tasks; no fabricated passenger relationships.
- At 516 x 794, map canvas increased from 357.4px to 539.4px (approximately 51%). Current platform, parent conversation and expanded collaborators are visible together.
- At 380 x 794, primary controls wrap and journey endpoints scroll without page-level horizontal overflow.
- Checked Full map, Locate current, menu bounds and Escape focus restoration in the browser.
- Node core/renderer tests: 154 passed. Rust UI/plugin contracts: 16 passed. Embedded resource remains within 160KiB.

The full map still permits horizontal and vertical scrolling for dense repositories. This change does not alter Git state, passenger counting or subagent evidence requirements.
