-- Handoff category split: action vs notify
--
-- Action handoffs are persistent until completed (the existing semantics).
-- Notify handoffs are ephemeral FYIs (introductions, watchdog drops, "I just
-- did a thing" announcements). They're filtered out of list_handoffs by
-- default and excluded from the action queue in check_in.
--
-- Read-time pruning: notify-class older than 7 days is filtered out at query
-- time. No background sweeper needed for v1; the rows stay in place for audit
-- and search history but never resurface in operational queries.

ALTER TABLE handoffs
    ADD COLUMN category TEXT NOT NULL DEFAULT 'action';

-- All existing handoffs are action-class. The DEFAULT covers them on backfill,
-- but pin the value explicitly so future schema changes don't surprise us.
UPDATE handoffs SET category = 'action' WHERE category IS NULL OR category = '';

-- Most queries filter by (status, category, to_machine). The existing
-- idx_handoffs_to_machine + idx_handoffs_status cover individual axes; this
-- composite index makes the action-queue lookup (the hot path in check_in
-- and list_handoffs) cheap regardless of table size.
CREATE INDEX idx_handoffs_category_status ON handoffs(category, status);
