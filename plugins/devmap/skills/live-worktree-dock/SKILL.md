---
name: live-worktree-dock
description: Show, open, or refresh the DevMap Live Worktree Dock when the user asks to inspect Agents on the current or other local Git worktrees. Do not use for general Git questions or cross-machine fleet monitoring.
---

# Live Worktree Dock

Use `devmap_open_dock` once when the user asks to show or open the visual Dock. The result is a read-only MCP App in Codex's side pane.

Use `devmap_dock_snapshot` when the user explicitly asks for a text-only refresh or inspection without opening the interface.

Report the scope honestly: the Dock covers local worktrees that share the current repository's Git common directory and only shows Agent state backed by available Presence records. Never claim cross-machine or organization-wide coverage.

In Codex, the plugin uses host-managed STDIO. Do not suggest a manual server command. If the tools are unavailable, say that plugin enablement, project trust, or managed MCP policy may be preventing activation.
