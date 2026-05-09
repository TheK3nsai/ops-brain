-- v2.0 — Rename knowledge.author_cc to author.
--
-- Pair migration to v2.0's agent-agnostic generalization. The `author_cc`
-- column was added in v1.6 (`20260408000001_add_knowledge_provenance.sql`)
-- with strict CC_TEAM allowlist validation in the handler. v2.0 drops the
-- allowlist in favor of free-form agent_name; the column rename makes the
-- semantics match.
--
-- Existing row values are preserved: 'CC-Stealth' etc. continue to be valid
-- free-form agent names. NULL stays NULL for pre-v1.6 rows. No data transform.
--
-- Immutability via the tool surface is preserved: handler-layer enforcement
-- (UpdateKnowledgeParams continues to omit `author`) — the DB itself does
-- not enforce immutability.

ALTER TABLE knowledge RENAME COLUMN author_cc TO author;

COMMENT ON COLUMN knowledge.author IS
    'Agent that created this knowledge entry (free-form slug). NULL for rows created before v1.6. Required on all new add_knowledge calls. Immutable via tool surface once set.';
