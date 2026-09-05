---
name: DevMap
description: A warm-light development metro for tracing repository topology, workspaces, and linked tasks.
colors:
  bg: "#fffdf8"
  bg-canvas: "#fffdf8"
  surface: "#ffffff"
  surface-raised: "#f2f5f8"
  line: "#778394"
  line-soft: "#dce1e6"
  text: "#17212f"
  muted: "#4f5b6c"
  main-rail: "#202124"
  branch-red: "#e32017"
  branch-blue: "#003688"
  branch-green: "#00782a"
  branch-yellow: "#ffd300"
  branch-cyan: "#0098d4"
  branch-magenta: "#9b0056"
  accent: "#005bbb"
  accent-soft: "#e9f1fb"
  success: "#176b38"
  warning: "#8a4b00"
  unknown: "#4f5b6c"
  focus: "#005bbb"
  keyline: "#4f5b6c"
typography:
  headline:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "1rem"
    fontWeight: 700
    lineHeight: 1.25
    letterSpacing: "-0.02em"
  title:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "0.9375rem"
    fontWeight: 700
    lineHeight: 1.45
  body:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "0.875rem"
    fontWeight: 400
    lineHeight: 1.45
  body-strong:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "0.875rem"
    fontWeight: 600
    lineHeight: 1.45
  label:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "0.75rem"
    fontWeight: 400
    lineHeight: 1.45
  status-label:
    fontFamily: "ui-sans-serif, system-ui, -apple-system, Segoe UI, sans-serif"
    fontSize: "0.75rem"
    fontWeight: 650
    lineHeight: 1.45
  mono:
    fontFamily: "ui-monospace, Cascadia Code, Consolas, monospace"
    fontSize: "0.75rem"
    fontWeight: 400
    lineHeight: 1.5
rounded:
  control: "6px"
  station: "50%"
spacing:
  compact: "4px"
  base: "8px"
  roomy: "12px"
  panel: "16px"
components:
  control:
    backgroundColor: "{colors.surface}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "8px 10px"
  control-hover:
    backgroundColor: "{colors.surface-raised}"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    rounded: "{rounded.control}"
    padding: "8px 10px"
  station:
    backgroundColor: "transparent"
    textColor: "{colors.text}"
    rounded: "{rounded.station}"
    size: "44px"
    height: "44px"
    width: "44px"
  workspace-cluster:
    backgroundColor: "transparent"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    padding: "4px 0"
  workspace-current:
    backgroundColor: "{colors.accent-soft}"
    textColor: "{colors.text}"
    typography: "{typography.title}"
    padding: "4px 8px"
  task-node:
    backgroundColor: "transparent"
    textColor: "{colors.text}"
    typography: "{typography.body-strong}"
    padding: "8px"
    width: "100%"
  task-onward:
    backgroundColor: "transparent"
    textColor: "{colors.accent}"
    typography: "{typography.label}"
  attention-link:
    backgroundColor: "transparent"
    textColor: "{colors.warning}"
    typography: "{typography.body}"
    padding: "8px 10px"
  selection-details:
    backgroundColor: "transparent"
    textColor: "{colors.text}"
    typography: "{typography.body}"
    padding: "16px 0"
---

# Design System: DevMap

## Overview

**Creative North Star: "The Development Metro"**

DevMap is a precise operational map, not a decorative transit poster. A warm paper-like canvas carries charcoal main rails, stable Underground-inspired branch colors, exact station rings, and quiet workspace/task structures. The visual system makes repository relationships traceable while keeping task ownership and unfinished work legible in a narrow Dock.

The interface is compact without becoming miniature. Flat tonal layers, one-pixel dividers, measured whitespace, and a single UI-sans family keep the graph dominant. Color identifies branch continuity; independent symbols and factual text communicate attention, freshness, and uncertainty.

**Key Characteristics:**

- Warm-light, flat operational surfaces
- Stable branch identity carried by rail color and pattern
- Precise circular commit stations and dashed association stems
- Compact workspace groups with visible task actions
- Explicit focus, uncertainty, and offscreen wayfinding

## Colors

Warm paper and white surfaces support a charcoal primary rail, six saturated branch rails, restrained blue interaction states, and dark semantic status colors.

### Primary

- **Charcoal Main** (`main-rail`): The primary integration rail and the strongest structural mark.
- **Operational Blue** (`accent`, `accent-soft`, `focus`): Current-workspace tint, links, active state, selection, and keyboard focus.

### Secondary

- **Underground Branch Set** (`branch-red`, `branch-blue`, `branch-green`, `branch-yellow`, `branch-cyan`, `branch-magenta`): Stable identities for non-main refs; repeat collisions are disambiguated with stroke pattern and text labels.

### Neutral

- **Warm Paper** (`bg`, `bg-canvas`): Page and topology world.
- **Clear Surface** (`surface`): Bordered controls and reversed skip-link text.
- **Raised Wash** (`surface-raised`): Quiet hover feedback without elevation.
- **Ink** (`text`): Primary operational copy.
- **Slate** (`muted`, `unknown`, `keyline`): Metadata, dashed associations, unknown states, and structural keylines.
- **Rules** (`line`, `line-soft`): Control borders and low-emphasis separators.

### Named Rules

**The Stable Line Rule.** Branch identity is derived from repository ID and full ref name; risk, freshness, and task state never recolor the rail.

**The Pale Rail Rule.** The yellow branch rail always receives the dark structural keyline on warm-light surfaces.

**The Independent Risk Rule.** Warning color accompanies a warning glyph and factual reason; it never substitutes for branch identity.

## Typography

**Interface Font:** UI Sans (with system UI fallbacks)

**Label/Mono Font:** UI Monospace (with Cascadia Code and Consolas fallbacks)

**Character:** Neutral, compact, and highly legible. Weight establishes hierarchy while the small type ramp preserves space for topology and full task names.

### Hierarchy

- **Headline** (700, `1rem`, 1.25): Dock title and selected-detail headings.
- **Title** (700, `0.9375rem`, 1.45): Workspace names, the primary identity inside each station group.
- **Body Strong** (600, `0.875rem`, 1.45): Task titles and other actionable operational names.
- **Body** (400, `0.875rem`, 1.45): Controls and readable supporting copy.
- **Label** (400, `0.75rem`, 1.45): Branch metadata, observation facts, state, counts, and wayfinding details.
- **Status Label** (650, `0.75rem`, 1.45): The compact current-workspace indicator.
- **Mono** (400, `0.75rem`, 1.5): Commit hashes and exact technical values in the inspector.

### Named Rules

**The Operational Minimum Rule.** Workspace names and task titles remain at 0.875rem or larger; meaningful metadata remains at 0.75rem and wraps before type shrinks.

**The Wrap Before Shrink Rule.** Task titles and factual reasons wrap; compact ref labels may ellipsize only when the full value remains available through accessible text.

## Layout

The Dock is a full-height flex column inside a centered container capped at 1440px. The header, attention summary, navigation, offscreen wayfinding, inspector, and help remain outside the scrollable topology viewport. At 619px and below, the outer padding tightens from 16px to 8px and detail grids collapse to one column.

The map uses one CSS-pixel world. Commit ranks advance horizontally, branch lanes stack vertically, and all rails, stations, crossing masks, labels, refs, and workspace stems are positioned from the same deterministic geometry. Horizontal and vertical overflow remain native. Workspace groups are measured before layout, capped at 320px wide, reduced to the viewport minus 80px when necessary, and limited to a scrollable 1024px height for very large task inventories.

Spacing follows a compact 4px/8px rhythm, with 12px and 16px reserved for panel relationships and larger separation. The implemented graph uses 96px column gaps and a corrected 48px lane-row gap, while 8px shelves keep workspace identity, refs, and commit labels grouped without overlap.

### Named Rules

**The One World Rule.** Rails, stations, crossing masks, labels, refs, and workspace stems share one CSS-pixel coordinate system.

**The Current Workspace Rule.** Initial framing keeps the current station, workspace name, active task preview, and locate control visible at narrow Dock widths.

**The Offscreen Is Not Unknown Rule.** A route outside the viewport remains navigable through endpoint wayfinding; only unavailable history receives a boundary treatment.

## Elevation & Depth

The system is flat. Warm paper, clear white controls, a pale blue current-workspace wash, and one-pixel rules create hierarchy without ambient elevation shadows. The only box-shadow is a one-pixel inner and outer keyline on yellow station rings; it protects contrast and does not imply depth.

### Shadow Vocabulary

- **Pale Rail Keyline** (`0 0 0 1px var(--keyline), inset 0 0 0 1px var(--keyline)`): Structural contrast for yellow station rings only.

### Named Rules

**The Flat Operational Rule.** Surfaces are separated by tone and one-pixel rules; shadows do not imply card hierarchy, and the station keyline is structural rather than elevation.

## Shapes

Commit stations are exact circles: a 44px interactive target contains an 18px center ring with a 3px branch-colored border. Dashed station borders indicate a history boundary. Rails are 5px strokes with round caps and joins; the yellow rail receives an 8px keyline beneath it. Workspace and ref attachments are 2px dashed stems. Crossings use local masks to create a true gap rather than a false junction.

Controls use gently compact 6px corners. Workspace groups, task rows, the inspector, and map boundaries stay rectilinear with divider lines rather than floating rounded cards.

### Named Rules

**The Station Ring Rule.** Commit nodes are circular rings centered on their rail; dashed rings mean boundaries, not ordinary commits.

**The Crossing Gap Rule.** A crossing without a commit node must show a local gap in the under-route and must never read as a connection.

## Components

### Controls

- **Shape:** Compact rounded rectangle (`6px`) with a one-pixel slate border on a clear surface.
- **Size:** Minimum 44px height with `8px 10px` internal padding.
- **Hover / Active:** Raised wash on hover; pale operational blue on active press.
- **Focus:** A 3px operational-blue outline with 2px offset; viewport focus moves inward by 3px.
- **Disabled:** Muted text and a waiting cursor; the control remains structurally present.

### Stations and Rails

- **Station:** 44px semantic button with an 18px circular ring centered on the rail.
- **Rail:** 5px branch-colored stroke; `solid`, `16 6`, and `2 7` patterns extend the identity set when colors repeat.
- **Association:** Thin dashed slate stem that attaches a workspace, ref, or boundary without posing as Git history.
- **Selection:** The selected graph object receives a persistent 2px operational-blue outline.

### Workspace Groups

- **Container:** Flat, transparent structure with one-pixel block dividers and `4px 0` shell padding.
- **Identity:** Workspace name at title weight, followed by muted branch metadata and stable short workspace identity.
- **Current State:** Pale operational-blue wash behind the identity row plus the stronger current label.
- **Facts:** Flexible metadata rows with warning and unknown glyphs shown beside their factual text.
- **Overflow:** A long expanded inventory scrolls inside the workspace group rather than clipping or increasing the world without bound.

### Transfer Platforms and Planned Arrivals

- A worktree platform attaches to its observed HEAD. Its compact face shows identity, observed passenger counts, integration state, shared-ancestor navigation, task rows, and delivery destination. Detailed facts remain available in a keyboard-accessible disclosure.
- One existing unarchived chat counts as one passenger, including its executing Agent. Completed, idle and unloaded chats still count. Archived/deleted records remain outside the passenger roster; legacy existence is unknown. Developing, waiting, finished and unknown activity are distinct from presence. A complete fresh inventory is required to confirm an unattended platform. Unattended uncommitted or unmerged work raises attention; clean included work only suggests cleanup review.
- Double-ring stations identify forks and merges present in retained commit topology. A current common ancestor never claims to be a recorded worktree creation point.
- Dashed journeys leave the platform for a labeled planned arrival area outside commit history. They add no commit, ancestry edge, or claimed completed merge. Missing retained target geometry remains distinct from an explicitly unavailable target.
- The self-contained resource budget is 144 KiB, including platform and arrival navigation; no external UI dependencies are added.

### Task Rows

- **Structure:** Full-width row with a soft top divider, 8px padding, and a minimum 44px target.
- **Title:** Strong body text that wraps for long and multilingual names.
- **Action:** A right-aligned underlined Open task cue remains visible at rest for verified task identities; unavailable navigation reads Inspect only in muted, non-underlined text.
- **State:** Agent and last-observed state remain separate muted labels; active state uses operational blue without changing the branch rail.
- **Disclosure:** Active and idle tasks expand together, while previous tasks live under a separate explicit history disclosure.

### Branch Index

- **Structure:** A native details disclosure containing wrapped 44px branch choices.
- **Identity:** Each row pairs a 28px rail swatch with the visible ref label; overflow ellipsizes the label while the full ref remains available.

### Selection Details

- **Container:** Secondary panel separated by a stronger top rule and `16px 0` padding.
- **Content:** A two-column definition grid becomes one column in compact Docks; exact hashes use the mono role.
- **Dismissal:** Details are dismissible and never duplicate the primary station/task hierarchy by default.

### Named Rules

**The Action Cue Rule.** Every verified task row exposes an underlined Open task cue at rest; inspect-only rows use muted, non-underlined copy.

**The 44 Pixel Rule.** Primary interactive rows and controls retain a 44px minimum hit target, including stations and coarse-pointer use.

## Do's and Don'ts

### Do:

- **Do** keep branch color and stroke pattern stable for the full repository/ref identity.
- **Do** pair every warning or unknown state with its own vector glyph and factual text.
- **Do** let long and multilingual task titles wrap while keeping their task action visible.
- **Do** preserve native scrolling, visible focus, keyboard panning, and explicit offscreen endpoint navigation.
- **Do** keep current-workspace identity and active task context readable in the first narrow viewport.

### Don't:

- **Don't** recolor a branch to express risk, activity, freshness, or integration state.
- **Don't** draw a junction at a crossing unless the graph contains a real shared commit node.
- **Don't** use floating merge arrows, disconnected origins, or inferred branch-parent relationships.
- **Don't** hide active tasks or risk reasons inside collapsed history.
- **Don't** add ambient shadows, gradients, decorative textures, or floating rounded cards to this flat operational world.
- **Don't** shrink operational names below 0.875rem or meaningful metadata below 0.75rem to make a narrow layout fit.
