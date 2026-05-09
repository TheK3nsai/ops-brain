-- v2.0 — Rename handoff machine columns to agent.
--
-- ops-brain becomes agent-agnostic in v2.0: any MCP-capable client can use it
-- as a team bus, not just the four CC instances. The handoff schema's
-- `from_machine` / `to_machine` columns were always free-text but the wording
-- assumed a host-per-agent fleet. Two agents on one host (e.g. CC-HSR plus a
-- delegated codex-hsr or gemini-hsr) need to be distinguishable; "machine"
-- actively hides that. Rename to `from_agent` / `to_agent`.
--
-- Existing row values are preserved: 'CC-Stealth', 'CC-Cloud', 'CC-HSR',
-- 'CC-CPA' continue to be valid free-form agent names. No data transform.
--
-- The category/status index is unaffected. Only the standalone
-- `idx_handoffs_to_machine` has a name embedding the old term.

ALTER TABLE handoffs RENAME COLUMN from_machine TO from_agent;
ALTER TABLE handoffs RENAME COLUMN to_machine TO to_agent;

ALTER INDEX idx_handoffs_to_machine RENAME TO idx_handoffs_to_agent;
