-- Dedupe scope: per recipient, not global.
--
-- The original arbiter index was on (dedupe_key) alone, so a producer
-- reusing one key across two target agents had its second filing silently
-- suppressed into the FIRST agent's open handoff — the second recipient
-- never saw it, and the response gave no hint the recipient differed.
--
-- Scoping the index to (dedupe_key, LOWER(to_agent)) makes the same key
-- file independently per recipient, matching the wake-poll mental model:
-- "this check, for this agent, is already open." LOWER() because agent
-- routing is case-insensitive everywhere else (allowlists, wake poll).
--
-- The predicate must stay in sync with the ON CONFLICT clause in
-- handoff_repo::create_machine_handoff.

DROP INDEX idx_handoffs_dedupe_key_open;

CREATE UNIQUE INDEX idx_handoffs_dedupe_key_open
    ON handoffs (dedupe_key, LOWER(to_agent))
    WHERE dedupe_key IS NOT NULL AND status IN ('pending', 'accepted');
