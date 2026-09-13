# Agent inventory synchronization — 2026-09-13

## Implemented Windows data flow

The viewer starts a repository-scoped collector. It reads Codex state_5.sqlite in read-only/query-only mode every five seconds, normalizes Windows extended paths, and selects unarchived tasks belonging to registered repository worktrees. Subagent source records are not counted as independent chats. Unsupported schema/source formats fail or mark coverage partial; a missing runtime observation is unknown, never evidence of inactivity.

Runtime activity comes from the already-running desktop through its local codex-ipc named pipe: initialize, versioned thread-owner-discovery, and following subscriptions. The adapter checks sender identity and stream version 11, accepts snapshots and contiguous revisions, and discards uncertain activity after gaps or disconnects. Only runtime status is retained; message bodies and tool output received in desktop snapshots are discarded. This is a private desktop compatibility adapter, not a claim of a stable public API.

One collector per repository holds agent-sync.lock. It publishes an atomic, versioned agent_inventory_cache row in the existing devmap.db. If no database exists, a shadow store is created without activating or importing the legacy domain backend. All new-version viewers independently read that cache; another viewer can take over collection after the writer exits. Cache writes do not increment domain generation or alter journal, route, or binding records. Client working-directory reports remain separately attributed and keep their original timestamps.

DEVMAP_AGENT_SYNC=off disables collection. DEVMAP_AGENT_SYNC_TRACE=1 enables local diagnostic stderr. Automatic collection currently targets Windows with the detected Codex v5 state database; other hosts retain the explicit inventory path. Cached observations still expire normally when acquisition stops.

## Connection evidence

Installed CLI: 0.154.0-alpha.6.2. Its generated protocol contains thread/list and thread/status/changed. The public daemon control-address query failed with Windows error 10050; connection to the desktop through that public entry point is NOT verified.

The desktop IPC probe DID connect, initialize, find the owner of task 01a081a1-751a-7473-aa3b-91995c128f7e, receive its runtime status active, and receive subsequent snapshot/patch notifications from the desktop instance. No replacement app-server or model turn was started for that proof. Probe artifacts are under target/verification/agent-sync-probe.

## Validation and limits

Two independent native MCP viewers, using different worktree sources, received eight real tasks and active status for the calling task without codex_tasks arguments. Both recovered automatically after the collector/viewer processes were restarted. Original task-bindings.jsonl, task-binding-watermarks.json and working-directory-reports.json hashes remained unchanged. A real 130-second browser observation checks that acquisition continues beyond the former 120-second expiry boundary.

Unit coverage checks metadata rename/archive/unarchive, subagent exclusion, extended Windows paths, incompatible metadata schema, full versus partial inventory replacement, unchanged working-directory report timestamps, independent SQLite readers, activity transitions and revision gaps. Fixture transitions are not evidence that a user task was actually archived or that the Codex desktop was restarted. Real desktop process restart and a natural active-to-idle event remain separate acceptance cases. Human visual acceptance is not replaced by these checks.

The UI uses task_inventory_synced_at for catalog freshness. The existing conservative task_observation timestamp can remain older when an execution-location report is old; it still governs occupancy/cleanup certainty. This prevents a stale location report from making a continuously refreshed task catalog look expired. The cache-only shadow exception in startup is restricted to generation zero with no records in any domain/provenance table; an unowned or populated migration shadow still requires recovery.

Final checks: 109 renderer tests; 5 Agent sync tests; 13 startup/recovery tests; 5 shared-proxy tests; 13 native UI contracts; Clippy with warnings denied. The final browser check includes one explicit calling-task directory report and confirms the other tasks continue to arrive automatically. The earlier 130-second test supplied no task inventory at all.
