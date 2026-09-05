# Route map and workspace inspector

Approved objective: make routes, junctions and the current station the visual subject. Keep Git facts and existing passenger semantics. Passenger detail must never determine rail geometry.

## Implementation

- Added an identity-only route projection. It normalizes workspace dimensions before projecting all retained commits and parent edges. Narrow sidebars read from history to present vertically; wide views retain horizontal reading.
- Replaced large workspace cards on the map with compact labels beside HEAD. Shared HEADs share one label and expose exact workspace choices. Current workspace identity remains visible in a shared label.
- Moved workspace conversations, direct collaborators and detailed facts into a collapsible, independently scrollable inspector beneath the map. Opening long rosters no longer expands the map or changes station coordinates.
- Full map and current-workspace views now use the same route representation, retaining readable labels and scrolling through dense histories.
- Kept future plans as separate dashed routes to planned arrival areas. Missing origins, destinations, history and passenger inventory remain uncertain; no historical creation or merge evidence is invented.
- Preserved task navigation, observation freshness, focus/selection restoration, shared HEAD selection and explicit v3 compatibility behavior.

## Verification

- Core/renderer tests: 158 passed, including invariant route geometry across detail sizes, exact task identity, long rosters, incomplete observations, v3 expansion and inspector focus restoration.
- Rust UI/plugin contract tests: 16 passed; embedded resource remains below 160KiB.
- Independent core review checked ordinary, boundary, empty and unborn histories in both orientations without losing nodes or routes.
- Browser acceptance uses the actual local repository and explicit task/subagent observations, at 516 x 794 and a wide viewport.

Scope deliberately excludes Git workflow execution, automatic merges, plugin installation and decorative transit imagery.
