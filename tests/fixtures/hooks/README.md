# Pinned hook fixtures

These event-specific payloads pin the documented Codex and Claude Code hook input
shapes used by the Phase 1B conformance gate, as checked on 2026-09-02. Content-
bearing official fields contain canaries so the test proves that native prompt,
tool, transcript, compaction, and assistant content does not cross the persistence
boundary.

Sources:

- https://developers.openai.com/codex/hooks
- https://docs.anthropic.com/en/docs/claude-code/hooks
