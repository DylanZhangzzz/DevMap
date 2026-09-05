# DevMap Rail View Design QA

## Evidence

- Source visual truth: `.superpowers/brainstorm/product-design/02-rail-view-source.png`
- Dark implementation reported by the user: the installed Dock at the start of this iteration
- Light implementation, iteration 1: `.superpowers/brainstorm/product-design/light-alignment-iteration-1.jpg`
- Combined comparison, iteration 1: `.superpowers/brainstorm/product-design/comparison-light-alignment-iteration-1.png`
- Structure-aligned implementation, iteration 3: `.superpowers/brainstorm/product-design/topology-alignment-final.jpg`
- Conversation panorama implementation, iteration 5: `.superpowers/brainstorm/product-design/conversation-panorama-final.jpg`
- Rail geometry before correction, iteration 6: `.superpowers/brainstorm/product-design/line-quality-before.jpg`
- Diagnostic source/before comparison: `.superpowers/brainstorm/product-design/comparison-line-quality-before.png`
- Rail geometry after correction, iteration 6: `.superpowers/brainstorm/product-design/line-quality-after.jpg`
- Final combined comparison: `.superpowers/brainstorm/product-design/comparison-line-quality-after.png`
- Focused rail comparison: `.superpowers/brainstorm/product-design/comparison-line-quality-detail.png`
- User-rejected shared-fork layout, iteration 7 before: `.superpowers/brainstorm/product-design/task-roster-before.png`
- Worktree-owned station layout, iteration 7 final: `.superpowers/brainstorm/product-design/task-roster-final.png`
- Direct rejected/final comparison, iteration 7: `.superpowers/brainstorm/product-design/comparison-task-roster-final.png`
- User-reported branch-first identity, iteration 8 before: `.superpowers/brainstorm/product-design/worktree-identity-before.png`
- Worktree-first identity, iteration 8 final: `.superpowers/brainstorm/product-design/worktree-identity-after.jpg`
- Focused identity comparison, iteration 8: `.superpowers/brainstorm/product-design/comparison-worktree-identity-final.png`
- Source pixels: 748 × 794. The source includes its design-host header and surrounding canvas.
- Final implementation pixels: 830 × 780 at the Codex in-app Browser desktop viewport. Browser device scale was not exposed; no density resampling was applied.
- Iteration 7 comparison pixels: 1,684 × 588. The rejected 762 × 499 capture and final 830 × 780 capture were placed together at native readable scale and cropped only to the shared topology region. The source mock remains the palette, typography, node-treatment, and rail-style reference; the rejected/final pair is the direct structural comparison because the live repository data differs from the synthetic source.
- Iteration 8 evidence pixels: 684 × 258 before, 1,280 × 720 after, and 1,640 × 350 focused comparison. The focused comparison preserves each image's aspect ratio and compares the same MAP-density worktree identity region; it is a semantic hierarchy check rather than a pixel-for-pixel layout comparison because the user capture is cropped.
- State: real local repository, light theme, MAP density, collapsed history disclosure, seven worktrees, and 12 linked Codex tasks (1 active, 1 idle, 10 history).

## Findings

No actionable P0/P1/P2 differences remain after the worktree-identity pass. The final comparison shows a single shared main timeline with one station per worktree, Worktree path as the primary node identity, Branch as secondary metadata, fork metadata attached to each lane rather than owning the lane group, vertical task/Agent stacks directly below each worktree, graphical return edges, an in-canvas title/control bar, and scoped horizontal navigation.

### P3 follow-up polish

- The source uses a faint square canvas grid. The implementation keeps a flat cool-neutral canvas because the shipped Dock is self-contained and the project contract prohibits external assets; no CSS-drawn or handcrafted substitute was introduced.
- The source includes a decorative `DM` brand tile. The implementation preserves the text brand only because no production logo asset is supplied; it does not fake the logo with a div, glyph, or inline SVG.

## Required fidelity surfaces

- **Fonts and typography:** compact system UI with monospace metadata preserves the source hierarchy. Primary text is near-black, secondary text is neutral gray, and branch labels keep readable weight and wrapping.
- **Spacing and layout rhythm:** the shell can use up to 1,440 px while the topology surface expands independently to the space required by its worktrees and conversations. Map surfaces use 16 px radii, white nodes use restrained 9 px radii, and shadows are limited to the map shell and interactive surfaces. The live data produces more vertical rows than the synthetic source, which is expected rather than design drift.
- **Colors and visual tokens:** page `#f4f5f7`, canvas `#fafbfc`, surface `#ffffff`, text/main rail `#202124`, and development rail `#1677ff` now follow the source's light visual system. Semantic green, amber, and red remain small-area accents.
- **Image quality and assets:** no repository-specific imagery is required. The missing decorative grid and brand tile are listed as P3 rather than replaced with prohibited code-drawn approximations.
- **Copy and content:** `DevMap · Rail View`, `Repository topology`, MAP/READ/FULL, Worktree path labels, secondary Branch labels, merge state, dirty state, and ahead/behind data remain explicit and truthful.

## Interaction and accessibility evidence

- MAP, READ, and FULL each set their own `aria-pressed="true"` state; the final state was returned to MAP.
- The main-timeline station markers retain a 14 px visual footprint while each button exposes a 44 × 44 px hit target.
- The legend is progressively disclosed in FULL only, keeping MAP and READ focused on topology.
- FULL mode reports no geometric overlap between `.worktree-stop` and `.task-stack` in any branch lane.
- Selecting a worktree sets `aria-current="true"`, updates the detail title to the Worktree path label, and exposes the Branch as a separate detail row.
- The current document reports 815/815 px `clientWidth`/`scrollWidth`, so the topology does not create document-level horizontal overflow.
- The current scoped topology viewport reports 2,760 px of content inside a 750 px viewport. Its native bottom scrollbar remains visible.
- Pointer drag moved the viewport from 300 px to 500 px, and keyboard Home, ArrowLeft/ArrowRight, and End reached the expected bounded positions.
- Seven visible worktree stations map to seven visible worktree clusters. Their measured station/cluster center difference is 0 px, with zero cluster overlap.
- The collapsed topology height is content-led at 603 px (`scrollHeight === clientHeight`). Expanding the historical disclosure renders all 10 history tasks and grows the topology to 951 px without internal vertical clipping (`scrollHeight === clientHeight`).
- Shift+wheel translation remains covered by the interaction contract and guarded at scroll boundaries, but the fixed Codex Browser automation surface could not synthesize the combined modifier-wheel gesture for an independent runtime measurement.
- Browser console warnings/errors: none.
- Contrast on white: primary text 16.10:1, muted text 4.70:1, success text 4.98:1, warning text 5.09:1, and dirty/error text 4.72:1.
- MAP, READ, and FULL were exercised in the current build. Pointer and keyboard selection resolve the exact active, idle, and history task without entering the panning state; the standalone environment truthfully falls back to `Open this task from Codex` when the host bridge is unavailable.
- Evidence limits for the current build: the fixed in-app Browser could not independently actuate the native scrollbar thumb, synthesize Shift+wheel, or resize to an exact 420 px viewport. These are interaction-evidence limits, not observed P0/P1/P2 visual defects; scrollbar presence and narrow-layout behavior remain covered by the UI contracts.

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

### Iteration 6

- Earlier P1: the main timeline station, fork label, and branch-line origin were calculated in three different containing blocks, producing a measured 36.41 px horizontal disagreement and visibly broken topology.
- Earlier P1: fork stems began near each branch group instead of at the main timeline, so later branches appeared as isolated vertical fragments rather than continuous ancestry paths.
- Earlier P2: branch endpoint circles were positioned by their outer edges rather than their centers, main and branch strokes used inconsistent weights, and the compact topology width allowed downstream fork stems to crowd preceding conversation cards.
- Fixes: measure each rendered main station once and project that coordinate into its fork group; derive the full vertical connector height from the main station to the last visible branch rail; place fork labels, branch origins, and conversation tracks from the same `--fork-x`; center both branch endpoint circles on the stroke; standardize main, branch, and connector strokes at 2 px with 12 px nodes; expand the topology-width budget so each fork has enough horizontal separation; recompute geometry after rendering, density changes, disclosure changes, and window resizing.
- Post-fix evidence: all five fork groups align within 0.39 px in MAP, READ, and FULL; narrow layout alignment remains within 0.13 px; continuous vertical connectors visibly join the main timeline to each branch; endpoint circles and return hooks remain centered; Browser warnings/errors are empty.
- Final visual evidence: `line-quality-after.jpg`, `comparison-line-quality-after.png`, and the native-pixel focused comparison `comparison-line-quality-detail.png`.

### Iteration 7

- Earlier P1: a single fork station visually owned multiple sibling worktrees, so the main rail encoded fork groups instead of the user-approved `main -> worktree -> task/Agent` hierarchy.
- Earlier P1: the active task stack appeared beside a sibling worktree under one shared station, making worktree ownership harder to read at a glance.
- Earlier P2: fork-group grid spans created a broad empty region and weakened the direct vertical relationship between each worktree station and its activity stack.
- Fixes: flatten the ordered branch lanes into one shared worktree sequence; render one timeline station and one vertically aligned cluster per worktree; keep fork metadata as secondary lane annotation; attach each task/Agent roster directly below its worktree; make the topology height follow the collapsed or expanded roster content while retaining scoped horizontal overflow.
- Post-fix visual evidence: `task-roster-before.png`, `task-roster-final.png`, and `comparison-task-roster-final.png`. The final capture visibly separates `codex/rail-view-design-alignment` and `codex/git-workflow-orchestrator-design` into distinct rail stations, and the task roster sits under its owning worktree.
- Post-fix geometry and interaction evidence: 7 stations, 7 clusters, 0 px maximum center difference, 0 overlaps, 2,760/750 px scoped horizontal extent, 815/815 px document width, 603/603 px collapsed height, 951/951 px expanded height, 12 task records, all three density modes, drag and keyboard panning, exact task selection, truthful standalone fallback, and no console warnings or errors.
- Focused comparison was not needed beyond the readable native-scale rejected/final topology crop: worktree labels, Agent/status text, station stems, and ownership relationships are legible in the combined image, while color and typography were unchanged from the already-passed iteration 6 source comparison.

### Iteration 8

- Earlier P1: Worktree nodes used `lane.branch` as their prominent visible label, so branch names such as `codex/rail-view-design-alignment` appeared to be the level-one identity even though each node represented a distinct Worktree.
- Earlier P2: detached Worktrees displayed only `detached HEAD`, hiding the directory that actually distinguishes them.
- Fixes: derive the primary visible identity from the last two segments of `workspace_path`; show `Branch · …` as secondary metadata in every density; use the same Worktree label in station accessibility names and selection-details headings; retain the complete path as the node tooltip and detail value.
- Post-fix visual evidence: `worktree-identity-after.jpg` and `comparison-worktree-identity-final.png`. The current node reads `ChatGPT/DevMap-phase-1a-worktree` with `Branch · codex/rail-view-design-alignment` beneath it, while Codex-managed detached Worktrees are differentiated by labels such as `5f3c/AI auto-git context` and `42c1/AI auto-git context`.
- Post-fix interaction evidence: all seven Worktree nodes expose distinct Worktree-first accessible labels; selecting `ChatGPT/AI auto-git context` sets `aria-current="true"`, uses that Worktree label as the detail heading, and renders `codex/git-workflow-orchestrator-design` in a separate Branch row. Selecting the active task still resolves the exact task and does not enter the panning state.

## Final result

final result: passed
