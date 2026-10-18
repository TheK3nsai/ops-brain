-- MCP create_handoff retry keys had no organic use; drop them. REST dedupe_key stays.
DROP INDEX IF EXISTS idx_handoffs_sender_idempotency;
ALTER TABLE handoffs
    DROP CONSTRAINT IF EXISTS handoff_idempotency_pair,
    DROP COLUMN IF EXISTS idempotency_key,
    DROP COLUMN IF EXISTS idempotency_request;
