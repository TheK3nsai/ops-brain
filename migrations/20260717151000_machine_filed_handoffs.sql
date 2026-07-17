-- Machine-filed handoffs (automation backbone, phase 1+2).
--
-- Non-interactive producers (monitors, cron sweeps, scripts) file handoffs
-- through a REST ingestion path (`POST /api/handoff`) authenticated by scoped
-- machine tokens, and local wake shims poll `GET /api/pending` to kick off
-- agent runs. Three structural additions:
--
--   1. `origin` — who filed the row. 'agent' (MCP tools, the default) or
--      'machine' (the REST ingestion path). Stamped server-side from the
--      caller class — callers never set it.
--   2. `dedupe_key` — caller-chosen idempotency key for recurring producers.
--      Uniqueness is enforced against OPEN rows only (pending/accepted): a
--      nightly sweep re-detecting the same failure suppresses into the
--      existing open handoff instead of piling up; once that handoff is
--      completed, the same key may file fresh.
--   3. `repeat_count` — server-maintained count of suppressed duplicates,
--      bumped (with `updated_at`) on each deduped POST. Distinguishes
--      "monitor still firing for 5 days" from "filed once and went quiet"
--      without per-occurrence history.
--
-- Recurrence itself stays on the producers' own schedulers (cron / Task
-- Scheduler) per the roadmap hard stop: ops-brain owns memory + coordination,
-- never execution timing. Dead-man detection is likewise out of scope —
-- producers must self-monitor their own liveness.

ALTER TABLE handoffs
    ADD COLUMN origin TEXT NOT NULL DEFAULT 'agent'
        CONSTRAINT handoffs_origin_check CHECK (origin IN ('agent', 'machine')),
    ADD COLUMN dedupe_key TEXT NULL,
    ADD COLUMN repeat_count INTEGER NOT NULL DEFAULT 0;

-- Arbiter index for the dedupe upsert. Partial: only open rows participate,
-- so a completed handoff releases its key. The predicate here must stay in
-- sync with the ON CONFLICT clause in handoff_repo::create_machine_handoff.
CREATE UNIQUE INDEX idx_handoffs_dedupe_key_open
    ON handoffs (dedupe_key)
    WHERE dedupe_key IS NOT NULL AND status IN ('pending', 'accepted');
