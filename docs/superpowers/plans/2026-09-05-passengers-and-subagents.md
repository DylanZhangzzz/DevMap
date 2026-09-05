# Passenger icons and observed collaborators

The user approved a passenger icon for unarchived chats and an expandable list of explicitly observed direct Subagents beneath the parent chat. This increment builds on the uncommitted overview changes on codex/devmap-journey-focus.

## Implemented contract

- The same small line icon marks the platform passenger summary, persistent journey count and present chat rows. Archived, deleted and unknown lifecycle rows do not use the current-passenger icon. Existing unknown/partial inventory qualifiers remain visible.
- A parent task can carry an optional `subagents` array through the existing map MCP tools. Input rows contain `id`, `name`, `status`, `observedAt`; the model exposes `id`, `display_name`, `status`, `observed_at`. Membership means explicitly observed direct collaboration under this exact parent task. It never creates another chat identity or increments passenger counts. Independently listed child chats still count once as chats.
- Each parent accepts up to 32 unique collaborators with 256-byte identity/name limits, explicit supported states and a valid observation timestamp. Invalid or duplicate observations reject the update. Missing relationship data is omitted; an empty observation clears the list. Git-only refresh retains the last observation without refreshing its timestamp.
- A native details control appears under the corresponding task only when there are observed collaborators. It is collapsed by default, preserves expansion through refresh, and exposes names/status as non-navigating text. Stale states say Last observed; unknown states stay unknown. All names use text rendering.
- The subtree sits beside the parent navigation control, never inside its link/button. Focus returns to the parent/workspace when an optional subtree disappears. Expanded rows participate in existing card measurement and bounded scrolling.
- The Skill documents when supported host/collaboration data may be attached and explicitly prohibits guessing relationships from cwd/title or reading private host databases. The optional subagents field is an addition to the existing task field allowlist.

## Evidence and limits

152 Node tests passed, including RED/GREEN checks for occupancy, lifecycle icons, invalid data, expansion, stale state and disappearance focus. The 70-test Rust map/model/viewer/plugin/UI suite passed; targeted follow-up tests also cover promotion of presence records, clearing/omitting collaborator observations and independent child chat counts. Code review findings on lifecycle icons and focus were fixed and re-reviewed.

Browser verification used the actual calling task and three collaborators returned by the supported collaboration tool, attached only to the verified calling task UUID. The host task listing reached its page limit, so passenger inventory was explicitly partial. No task link was activated for screenshot capture.

Final screenshot: `target/verification/passengers-subagents-final.png`. It shows the real parent task and three explicitly observed completed collaborators. Opening the group leaves the platform's 16 observed chats unchanged; the incomplete inventory remains qualified as unknown/partial.

This implementation consumes supported explicit relationship observations. The normal task-list API does not expose every task's collaborator tree, and historical native hook parent fields are not automatically replayed into this new UI. Unknown/unavailable relationships therefore remain undisplayed. It does not claim exhaustive Subagent discovery across all chats or machines. Native event integration can be added when needed; no speculative tree is fabricated now.

The embedded UI remains within the existing 160 KiB regression budget and 512 KiB transport cap. This increment is not committed, pushed, merged or installed by this task.
