-- v1.6: knowledge provenance (who wrote it, which incident produced it).
--
-- v1.5 established that local is the source of truth for everything except
-- shared cross-CC knowledge. That one shared artifact was missing provenance:
-- stale cross-scope entries looked identical to fresh local ones in search
-- results, a stealth bug waiting to bite. This migration closes that gap.
--
-- Two new nullable columns on `knowledge`:
--
-- 1. `author_cc` (TEXT) — which CC created the entry. v1.5 dropped the
--    `cc_identities` table, so this is populated from a per-call param on
--    `add_knowledge`, validated against the static CC_TEAM allowlist in
--    `src/tools/cc_team.rs` (CC-Cloud, CC-Stealth, CC-HSR, CC-CPA). No
--    DB-side membership table; the allowlist lives in Rust source and
--    moves with the code. Becomes required on all future add_knowledge
--    calls via an allowlist check in the handler.
--
-- 2. `source_incident_id` (UUID) — the incident that produced this
--    knowledge entry, if any. Links gotchas back to the incidents that
--    taught us those gotchas. ON DELETE SET NULL so incident cleanup
--    doesn't cascade-delete the knowledge we learned from it. Can be
--    set at create time or added post-hoc via update_knowledge.
--
-- Existing rows backfill to NULL for both columns — no forced re-stamping,
-- no fabricated provenance. Honest legacy state.
--
-- Staleness warnings are computed at read time from `last_verified_at` and
-- `created_at` — no new column, no background job, no drift.
--
-- Immutability: `author_cc` is set once at creation and NEVER updatable
-- via the tool surface (enforced at the handler layer, not the DB layer —
-- direct SQL updates are still possible for emergency correction).

ALTER TABLE knowledge
    ADD COLUMN IF NOT EXISTS author_cc TEXT NULL;

ALTER TABLE knowledge
    ADD COLUMN IF NOT EXISTS source_incident_id UUID NULL
        REFERENCES incidents(id) ON DELETE SET NULL;

COMMENT ON COLUMN knowledge.author_cc IS
    'CC that created this knowledge entry (CC-Cloud, CC-Stealth, CC-HSR, CC-CPA). NULL for rows created before v1.6. Required on all new add_knowledge calls. Immutable via tool surface once set.';

COMMENT ON COLUMN knowledge.source_incident_id IS
    'Incident that produced this knowledge entry, if any. NULL if the entry is standalone or pre-dates v1.6. FK with ON DELETE SET NULL — cleaning up incidents does not cascade-delete the lessons learned from them.';
