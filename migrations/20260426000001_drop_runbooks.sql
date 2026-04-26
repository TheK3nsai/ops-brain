-- Drop the runbooks feature in its entirety.
--
-- Rationale: ops-brain is the team bus, not a brain. Runbooks describe
-- system-specific procedures that drift away from the systems they document
-- when stored centrally. Each repo's CLAUDE.md / docs is the source of truth
-- for its own procedures; cross-CC durable knowledge already lives in the
-- `knowledge` store.
--
-- Drop order respects FK dependencies (CASCADE handles indexes, triggers,
-- FTS columns, pgvector embeddings, and the incident_runbooks /
-- runbook_executions junction tables automatically).

DROP TABLE IF EXISTS incident_runbooks CASCADE;
DROP TABLE IF EXISTS runbook_executions CASCADE;
DROP TABLE IF EXISTS runbook_servers CASCADE;
DROP TABLE IF EXISTS runbook_services CASCADE;
DROP TABLE IF EXISTS runbooks CASCADE;
