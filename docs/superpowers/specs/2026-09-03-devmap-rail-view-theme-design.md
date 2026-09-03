# DevMap Rail View and Color System Design

**Date:** 2026-09-03

**Status:** Approved

**Visual source:** `.superpowers/brainstorm/product-design/02-rail-view-source.png`

**Existing-screen evidence:** `.superpowers/brainstorm/product-design/01-current-ui.png`

## 1. Goal

Replace the Dock's nested status-card composition with a Git-topology-first Rail View. A user should be able to identify `main`, parallel working branches, exact fork points, worktree ownership, merge readiness, dirty state, and linked Agent activity in that order without reading every card.

The redesign must retain every existing safety property: bounded `devmap/dock/2` validation, no HTML injection, no external assets, local-only operation, portable MCP/browser transport, accessible native controls, and honest unknown states.

## 2. Product-design audit

The current screen has four structural problems:

1. Common-base, worktree, task, and return status all use large bordered cards with comparable weight, so containment is stronger than ancestry.
2. Root blue, development cyan, branch purple, merged green, open amber, and dirty red occupy large surfaces simultaneously. The palette communicates category and state with the same intensity.
3. Long branch and task names wrap across multiple lines inside narrow columns, increasing row height and breaking scan continuity.
4. Repeated `Common base with main` headers and tall merge-state panels consume the space that should show branch direction and fork/return paths.

Visible accessibility risks are low-contrast secondary metadata, color-heavy state encoding, and focus order across a visually fragmented three-column row. Screenshot evidence cannot establish keyboard behavior, screen-reader output, or contrast ratios; those require implementation QA.

## 3. Approved information architecture

The Dock uses four layers:

1. **Repository rail:** `main` or the resolved root integration branch is pinned as the top horizontal baseline.
2. **Branch rails:** each visible working branch is a separate horizontal lane oriented in the same direction as `main`. The lane begins at its exact common-base station and ends at its HEAD/return state.
3. **Worktree stop:** the associated worktree appears as one compact stop on its branch rail. Dirty and ahead/behind facts are short text badges, not full-height panels.
4. **Activity detail:** linked Agents are summarized as a count in MAP mode, show task titles in READ mode, and reveal task metadata in FULL mode or the shared detail inspector.

Worktrees remain distinct even if they share a branch name. Worktrees that share the same exact fork point share one station label but keep separate branch/worktree lanes.

## 4. Density modes

The default camera is **MAP**:

- integration and branch rails;
- branch/worktree identity;
- fork hash;
- merge state, dirty marker, and Agent count.

**READ** adds linked task titles and current activity state while keeping metadata hidden.

**FULL** adds host/capture metadata and preserves the existing detailed selection inspector. Switching mode is local UI state only and is not persisted.

The controls are native buttons with `aria-pressed`; the active mode has text, fill, and border differences. Keyboard order is MAP, READ, FULL, graph nodes, collapsed-group control, then detail controls.

## 5. Branch visibility and collapse

At most six ordinary branch/worktree lanes are expanded by default. Priority is deterministic:

1. current worktree;
2. dirty worktree;
3. not-merged worktree;
4. worktree with a `starting`, `working`, or `waiting` Agent;
5. clean merged/terminal worktree;
6. stable branch name, workspace path, then worktree ID.

When more than six lanes exist, remaining merged or inactive lanes collapse behind one native button: `+ N merged / inactive branches`. Activating it reveals all lanes and changes the label to `Collapse inactive branches`. No lane is removed from the read model.

At narrow widths below 620 px, the same DOM order becomes a vertical tree. The document never requires horizontal scrolling.

## 6. Color system

The approved dark palette uses restrained graphite surfaces and a single structural accent:

- page background: near-black graphite;
- primary surface: dark slate;
- raised/selected surface: slightly lighter slate;
- primary text: cool off-white;
- secondary text: blue-gray;
- topology accent: medium blue, used for rails, selection, and focus;
- merged: green, used only for a short stroke, dot, or label;
- not merged/review: amber, used only for a short stroke, dot, or label;
- dirty/error: coral red, used only for a dot and text;
- unknown: neutral dashed stroke plus explicit `Unknown` text.

Purple and cyan card fills are removed. State is never conveyed by color alone. Large surfaces remain neutral, borders are low contrast, and shadows are reserved for the selected worktree stop and detail drawer.

## 7. Component contract

The self-contained asset keeps the existing transport and validator code. Rendering changes use these semantic contracts:

- `.topology-canvas`: repository graph container;
- `.integration-rail`: root/development baseline;
- `.branch-rails`: branch-lane collection for a target;
- `.branch-rail`: one horizontal working branch;
- `.fork-node`: exact common-base station;
- `.worktree-stop`: selectable worktree identity;
- `.agent-summary` and `.task-node`: progressive Agent detail;
- `.return-state`: explicit merged/not-merged/unknown state;
- `.density-switch`: MAP/READ/FULL controls;
- `.collapsed-branches`: disclosure control for hidden inactive lanes;
- `#selection-details`: shared inspector retained below the graph.

The graph uses semantic HTML and CSS borders/backgrounds. No external icon, font, image, inline SVG, storage, or network dependency is added.

## 8. Interaction and state

Selecting an integration rail, fork point, worktree, or task keeps the existing detail behavior and model-context update boundary. The selected graph node receives `aria-current="true"` and a visible focus/selection ring.

Refresh behavior, offline aging, host-backed task refresh, copy-hash behavior, and snapshot validation remain unchanged. A render caused by a new snapshot preserves the selected density mode and collapsed/expanded preference for the current page lifetime, but invalid or missing IDs clear node selection safely.

## 9. Testing and acceptance

Automated contracts must prove:

- the new semantic classes and MAP/READ/FULL controls exist;
- default density is MAP and buttons expose `aria-pressed`;
- branch collapsing uses a six-lane constant and prioritizes current/dirty/unmerged/active lanes;
- each worktree lane retains explicit merged/not-merged/unknown and dirty text;
- existing safe DOM, bounded validation, transport, refresh, clipboard, responsive, and reduced-motion contracts remain;
- the self-contained asset stays under 128 KiB.

Browser QA must cover the real Viewer at 375, 594, 736, and 1,024 CSS px; verify no document overflow, no overlap, usable focus, density switching, branch disclosure, selection details, refresh, and zero console errors. Product Design QA compares the selected Rail View source with the rendered implementation and blocks delivery until no P0/P1/P2 issues remain.
