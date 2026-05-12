-- v3.1 — Handoff threading + commit linkage.
--
-- Two friction patterns retired:
--
--   1. "Where's the reply to my handoff?" — currently relies on body-text
--      grep + include_notify=true, which has bitten the fleet at least once
--      (CC-Cloud's BGP probe reply, 2026-05-05). `in_reply_to` makes
--      threading structural so list_replies_to_me() returns it directly.
--
--   2. "Did the work in handoff X actually land in main?" — currently lives
--      in body text ("completed in abc1234"). `commit_hash` carries the
--      ref structurally; `mark_merged` flips status + records merge_commit
--      and merged_at when the bundle containing commit_hash reaches main.
--
-- No backfill: new rows opt in via new params; historical rows stay NULL on
-- the new columns. ON DELETE SET NULL on the in_reply_to FK keeps replies
-- intact if the parent is deleted.

ALTER TABLE handoffs
    ADD COLUMN in_reply_to UUID NULL REFERENCES handoffs(id) ON DELETE SET NULL,
    ADD COLUMN commit_hash TEXT NULL,
    ADD COLUMN merge_commit TEXT NULL,
    ADD COLUMN merged_at TIMESTAMPTZ NULL;

-- Hot path: list_replies_to_me joins on r.in_reply_to = parent.id, then
-- orders by r.created_at DESC. Partial index keeps non-reply rows out of
-- the lookup entirely.
CREATE INDEX idx_handoffs_in_reply_to
    ON handoffs (in_reply_to, created_at DESC)
    WHERE in_reply_to IS NOT NULL;

-- Reverse lookup: "did this commit land in any handoff?" Partial index
-- keeps the common case (no commit_hash) cheap.
CREATE INDEX idx_handoffs_commit_hash
    ON handoffs (commit_hash)
    WHERE commit_hash IS NOT NULL;
