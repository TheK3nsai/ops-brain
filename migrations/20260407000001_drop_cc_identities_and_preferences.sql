-- v1.5: ops-brain is a team bus, not a brain.
--
-- Drop two tables that stored state CCs already know locally:
--
-- 1. cc_identities — held self-authored CC scope. Identity and scope live in
--    each CC's per-machine CLAUDE.md, not in a database row. Storing it here
--    created drift between the source of truth (CLAUDE.md) and a stale copy.
--    The table held at most four rows of free-text scope descriptions, all
--    of which are also in the respective CLAUDE.md files.
--
-- 2. preferences — held tool-default settings (e.g. compact=true). These are
--    per-CC local config and belong in each CC's CLAUDE.md or in per-call
--    parameters, not in a centrally-shared row that quietly mutates other
--    CCs' tool defaults.
--
-- Data loss is intentional and zero — neither table held anything that
-- isn't authoritatively stored locally.

DROP TABLE IF EXISTS cc_identities;
DROP TABLE IF EXISTS preferences;
