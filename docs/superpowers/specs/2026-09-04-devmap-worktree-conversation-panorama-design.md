# DevMap Worktree Conversation Panorama Design

Date: 2026-09-04

## Status

Approved direction, pending written-spec review.

## Problem

The current Rail View correctly represents Git topology, but its visual hierarchy stops at the worktree. Conversation and agent information is reduced to a small summary, hidden entirely in MAP density, and separated from the branch identity in denser modes. The fixed 980 px shell and hidden horizontal overflow then force worktree, activity, and return-state content into too little space.

The result answers “which worktrees exist?” but does not answer the primary operational questions quickly enough:

1. Which conversations are attached to this worktree?
2. Which observed agent is active in each conversation?
3. What is the relationship between that activity and the repository topology?

## Product outcome

Preserve the worktree as the stable first-level object while making its conversations and observed agent identity unmistakable second- and third-level information. Let the topology expand horizontally instead of compressing it into the viewport.

## Information hierarchy

### Level 1: Worktree lane

Each worktree owns one horizontal band. Its pinned identity block contains:

- branch or detached-HEAD label;
- current-worktree state;
- dirty file count;
- ahead/behind and merge target;
- abbreviated workspace path in READ and FULL density.

The worktree remains the selection and routing boundary for repository context.

### Level 2: Conversation tracks

Every linked conversation is rendered as an individual, clickable node inside its worktree band. Conversation title is the dominant text within the activity area. Conversations are never hidden in MAP density.

Ordering is deterministic:

1. starting, working, or waiting;
2. idle;
3. completed or stale;
4. newest `last_event_at` first within each group;
5. `session_id` as the stable tie-breaker.

The default view shows every active conversation and the three most recent inactive conversations per worktree. Older inactive conversations collapse into one `+N historical conversations` control. Expanding that control lays the historical nodes out along the same horizontal canvas; it does not create an overlapping vertical card stack.

An uninstrumented worktree uses a quiet `No linked conversation` label rather than a card that competes with real activity.

### Level 3: Observed agent identity

Each conversation node shows the truthful observed identity already available in `DockChat`:

- `actor_id`;
- effective state from `host_status` or presence `status`;
- host;
- capture-confidence warning when capture is incomplete.

ACTIVE-like states receive the strongest semantic emphasis. IDLE, completed, stale, and unknown states remain readable but visually recede.

The current model exposes one observed actor identity per conversation. The UI must not imply a complete multi-agent roster or fabricate subagent identity when only the generic observed Codex actor is available.

## Panoramic topology

### Coordinate system

- The vertical axis contains worktree bands.
- The horizontal axis contains repository stations, conversation nodes, and the return target.
- The main integration rail spans the full topology width.
- Worktree branch rails begin at their real fork station and extend through their conversation activity to the merge-return edge.

The topology surface derives a bounded minimum width from the number of fork stations and the maximum visible conversation count. It must be at least the viewport width and may extend to several thousand pixels for a large repository.

### Viewport regions

The Dock is split into two coordinated regions:

1. A sticky left identity column for BASE and WORKTREE information.
2. A horizontally scrollable topology viewport for branch rails, conversations, agents, and TARGET.

The toolbar and density controls remain outside the moving surface. The main-rail label and worktree identity blocks stay visible while the topology moves horizontally. Target labels may remain at the far right of the topology rather than being compressed into the initial viewport.

### Horizontal navigation

The topology viewport supports all of the following:

- native trackpad horizontal scrolling;
- Shift + mouse wheel horizontal scrolling;
- click-and-drag panning from non-interactive canvas space;
- a persistent native horizontal scrollbar at the bottom of the viewport;
- keyboard scrolling when the viewport itself has focus.

Drag panning must not begin from buttons, links, text-selection areas, or the scrollbar. The cursor changes between `grab` and `grabbing`, and reduced-motion preferences disable animated scrolling.

The page itself must not gain document-level horizontal overflow; only the topology viewport scrolls.

## Density modes

Density changes detail, not object visibility:

- **MAP:** worktree identity, conversation title, observed agent, and active/idle state.
- **READ:** MAP content plus workspace path, host, and relative activity time.
- **FULL:** READ content plus session identifier, association source, capture state, and legend.

Conversation presence is therefore visible in every density. Density controls never replace activity with a numeric summary.

## Selection and routing

- Selecting a worktree updates the attached inspector with Git and conversation summary information.
- Selecting a conversation updates the inspector with conversation title, session, actor, host, state, association, and capture status.
- When a truthful `route_id` exists, selection continues to publish portable model context for opening the corresponding Codex task.
- Missing routes remain explicit; the interface does not invent a destination.

## Responsive behavior

At narrow widths, the worktree identity column becomes a compact sticky label rather than converting the topology back into a vertically stacked card list. Horizontal navigation remains the primary way to inspect the rail.

Touch input uses native horizontal scrolling. Interactive targets expose at least a 44 × 44 px hit area without enlarging their visible rail markers.

## Data and architecture boundary

The existing `devmap/dock/2` model already contains the required conversation title, `actor_id`, status, host, timestamps, capture state, and route identity. This iteration is a Dock rendering and interaction change; it does not change the Rust serialization schema.

The implementation may add pure JavaScript helpers for:

- conversation ordering and disclosure;
- topology-width calculation;
- safe pointer-drag state;
- Shift-wheel translation.

All rendered content continues to use DOM text nodes. No untrusted value may enter `innerHTML`, style text, selectors, or executable code.

## Failure and uncertainty states

- No linked task inventory: show `No linked conversation` and keep the Git rail usable.
- Generic observed actor: show the provided actor value and capture warning without claiming a named subagent.
- Stale or incomplete presence: preserve the conversation node with an explicit stale/incomplete label.
- Invalid snapshot: keep the last valid revision visible and report the connection state.
- Excessive data: respect existing bounded model limits and collapse only older inactive conversations.

## Test strategy

Automated UI contract tests must prove:

- worktree remains the first-level lane identity;
- conversation nodes are present in MAP, READ, and FULL;
- actor identity and activity state are rendered on each conversation;
- inactive-history disclosure is deterministic and expandable;
- the topology owns horizontal overflow while the document does not;
- pointer drag ignores interactive descendants;
- Shift-wheel and keyboard scrolling target the topology viewport;
- existing validation, safe text rendering, selection, refresh, and density contracts remain intact.

Browser Design QA must verify:

- no overlap among worktree, conversation, agent, and return-state content;
- active conversation and agent can be identified without opening the inspector;
- drag, trackpad/scrollbar, Shift-wheel, density controls, worktree selection, and conversation selection work;
- the left identity column remains visible during horizontal movement;
- there is no console error or document-level horizontal overflow;
- comparison evidence includes the approved Rail View source, the user-reported cramped state, and the corrected panoramic state.

## Out of scope

- Changing the Dock schema or capture pipeline.
- Inferring unobserved agents or reconstructing historical conversations.
- Adding a complete commit graph or per-commit file diff browser.
- Opening, mutating, stopping, or moving Codex tasks from the Dock.
- Replacing the approved light visual system.

