# DevMap Rail View Design QA

## Evidence

- Source visual truth: `.superpowers/brainstorm/product-design/02-rail-view-source.png`
- Dark implementation reported by the user: the installed Dock at the start of this iteration
- Light implementation, iteration 1: `.superpowers/brainstorm/product-design/light-alignment-iteration-1.jpg`
- Combined comparison, iteration 1: `.superpowers/brainstorm/product-design/comparison-light-alignment-iteration-1.png`
- Structure-aligned implementation, iteration 3: `.superpowers/brainstorm/product-design/topology-alignment-final.jpg`
- Conversation panorama implementation, iteration 5: `.superpowers/brainstorm/product-design/conversation-panorama-final.jpg`
- Final combined comparison: `.superpowers/brainstorm/product-design/comparison-conversation-panorama-final.png`
- Source pixels: 748 × 794. The source includes its design-host header and surrounding canvas.
- Implementation pixels: 1,264 × 710 at the Codex in-app Browser's current desktop viewport and device scale.
- Comparison normalization: source retained at 748 × 794; implementation content cropped from x=164 to x=1,085 and normalized to 748 × 577 beside the source. The comparison is for palette, typography, node treatment, and rail styling rather than a pixel-identical content layout because the source and live repository contain different worktree data.
- State: real local repository, light theme, MAP density, six worktrees, and one live Codex task supplied from the local active/idle task inventory.

## Findings

No actionable P0/P1/P2 differences remain after the structure pass. The final comparison shows a single shared main timeline, spaced fork stations, branch lanes that start from those stations, compact rail-attached worktree nodes, graphical return edges, an in-canvas title/control bar, and the attached selection inspector.

### P3 follow-up polish

- The source uses a faint square canvas grid. The implementation keeps a flat cool-neutral canvas because the shipped Dock is self-contained and the project contract prohibits external assets; no CSS-drawn or handcrafted substitute was introduced.
- The source includes a decorative `DM` brand tile. The implementation preserves the text brand only because no production logo asset is supplied; it does not fake the logo with a div, glyph, or inline SVG.

## Required fidelity surfaces

- **Fonts and typography:** compact system UI with monospace metadata preserves the source hierarchy. Primary text is near-black, secondary text is neutral gray, and branch labels keep readable weight and wrapping.
- **Spacing and layout rhythm:** the shell can use up to 1,440 px while the topology surface expands independently to the space required by its worktrees and conversations. Map surfaces use 16 px radii, white nodes use restrained 9 px radii, and shadows are limited to the map shell and interactive surfaces. The live data produces more vertical rows than the synthetic source, which is expected rather than design drift.
- **Colors and visual tokens:** page `#f4f5f7`, canvas `#fafbfc`, surface `#ffffff`, text/main rail `#202124`, and development rail `#1677ff` now follow the source's light visual system. Semantic green, amber, and red remain small-area accents.
- **Image quality and assets:** no repository-specific imagery is required. The missing decorative grid and brand tile are listed as P3 rather than replaced with prohibited code-drawn approximations.
- **Copy and content:** `DevMap · Rail View`, `Repository topology`, MAP/READ/FULL, branch names, merge state, dirty state, and ahead/behind data remain explicit and truthful.

## Interaction and accessibility evidence

- MAP, READ, and FULL each set their own `aria-pressed="true"` state; the final state was returned to MAP.
- The main-timeline station markers retain a 14 px visual footprint while each button exposes a 44 × 44 px hit target.
- The legend is progressively disclosed in FULL only, keeping MAP and READ focused on topology.
- FULL mode reports no geometric overlap between `.worktree-stop` and `.task-stack` in any branch lane.
- Selecting a worktree sets `aria-current="true"` and updates the detail title to the selected branch.
- The final desktop viewport reports `scrollWidth === clientWidth` (1,265 px), so the topology does not create document-level horizontal overflow.
- The scoped topology viewport reports 1,750 px of content inside a 1,200 px viewport. Its native bottom scrollbar remains visible without widening the document.
- Pointer drag moved the viewport from 0 px to approximately 411 px, and the `is-panning` state cleared on release. Keyboard End moved it to approximately 546 px of a 550 px maximum; Home returned it to the start.
- All six sticky worktree identities remained at the same 25 px left position while the activity and target columns scrolled behind their opaque mask.
- At a 420 px Browser viewport, the document remained 405 px wide while the scoped topology viewport retained 1,750 px of scrollable content and a 1,494 × 3 px visible branch rail.
- Shift+wheel translation is covered by the interaction contract and guarded at scroll boundaries; the Browser automation surface could not synthesize the combined modifier-wheel gesture for an independent runtime measurement.
- Browser console warnings/errors: none.
- Contrast on white: primary text 16.10:1, muted text 4.70:1, success text 4.98:1, warning text 5.09:1, and dirty/error text 4.72:1.

## Iteration history

### Iteration 1

- Earlier P1: the implementation used a dark graphite theme even though the selected source was light.
- Earlier P1: integration and development rail color roles were reversed relative to the source.
- Earlier P2: cards and controls used dark filled surfaces rather than white nodes on a cool-neutral canvas.
- Fixes: changed the document color scheme to light; mapped the integration rail to near-black and development rails to blue; changed nodes, controls, details, and map shell to white/neutral surfaces; reduced elevation and saturated fill area.
- Post-fix evidence: `comparison-light-alignment-iteration-1.png`.

### Iteration 2

- Earlier P2: the dirty/error text color measured 3.72:1 on white at small UI text sizes.
- Fix: darkened the error token from `#eb476a` to `#d92d54`, producing 4.72:1 contrast on white.
- Post-fix evidence: `comparison-light-alignment-final.png`.

### Iteration 3

- Earlier P1: the screen remained a vertically grouped list rather than the source's single topology coordinate system.
- Earlier P1: merge return state remained a large status block rather than a graph edge.
- Earlier P2: worktree cards and row spacing were too tall, the map title and density controls sat outside the graph shell, and the canvas lacked positioned main-rail stations.
- Fixes: moved `Repository topology`, summary metrics, density controls, and legend into the map shell; added semantic BASE/BRANCH RAILS/ACTIVITY/TARGET guides; distributed clickable fork stations across one main timeline; started each branch rail from its station; replaced large return blocks with compact labels and green return hooks; hid workspace paths in MAP mode; compressed all six real worktrees into the initial graph viewport; limited current-worktree emphasis to its node.
- Post-fix evidence: `comparison-topology-alignment-final.png`.

### Iteration 4

- Earlier P2: FULL density placed workspace and linked-task content in the same grid area, allowing content to overlap on populated lanes.
- Earlier P2: global touch sizing risked turning the compact main-timeline stations into oversized visible circles, and the legend competed with topology controls in every density.
- Fixes: moved linked tasks onto a dedicated second grid row while preserving the return column; separated the 44 × 44 px station hit target from its 14 px visual marker; limited the legend to FULL density.
- Post-fix evidence: browser geometry reports zero workspace/task overlap and `scrollWidth === clientWidth`; the refreshed final screenshot and combined comparison show MAP density after the correction.

### Iteration 5

- Earlier P1: worktrees and branches carried the strongest visual weight, but linked conversations and their Agents were not legible as the active execution layer.
- Earlier P1: the topology was compressed into the available Dock width, leaving insufficient horizontal space for multiple conversations under one worktree.
- Earlier P2: horizontal navigation lacked explicit drag and keyboard affordances, and narrow Dock widths collapsed the graph instead of preserving its spatial model.
- Fixes: kept Worktree as the level-one row; placed ordered conversation nodes directly on each Worktree rail; made conversation title, Agent identity, and ACTIVE/IDLE state visible in MAP; exposed progressively richer host/session/capture details in READ and FULL; retained all active plus three recent inactive conversations and grouped older history behind `+N historical conversations`; added a bounded 1,080–6,400 px topology surface with scoped native overflow; made Worktree identities sticky; added background pointer-drag, Shift+wheel, Arrow/Home/End navigation, and grab/grabbing feedback; preserved the horizontal rail at narrow widths; reduced empty lanes to one quiet `No linked conversation` node.
- Post-fix evidence: the real active task `查找今天上午开发结果 (2)` appears under its owning Worktree with `Agent · codex` and `ACTIVE`; MAP/READ/FULL progressive disclosure, selection details, sticky geometry, pointer drag, keyboard pan, narrow-width behavior, and zero console errors were verified in the Codex in-app Browser.
- Final visual evidence: `conversation-panorama-final.jpg` and `comparison-conversation-panorama-final.png`.

## Final result

final result: passed
