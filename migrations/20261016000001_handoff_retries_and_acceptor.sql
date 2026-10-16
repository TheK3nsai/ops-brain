-- MCP retries are scoped to the sender and remain reserved after completion.
-- Keep the original request separate from mutable lifecycle/thread fields.
ALTER TABLE handoffs
    ADD COLUMN accepted_by TEXT,
    ADD COLUMN idempotency_key TEXT,
    ADD COLUMN idempotency_request JSONB,
    ADD CONSTRAINT handoff_idempotency_pair CHECK (
        (idempotency_key IS NULL) = (idempotency_request IS NULL)
    );

CREATE UNIQUE INDEX idx_handoffs_sender_idempotency
    ON handoffs (LOWER(from_agent), idempotency_key)
    WHERE idempotency_key IS NOT NULL;

CREATE INDEX idx_handoffs_broadcast_acceptor
    ON handoffs (LOWER(accepted_by))
    WHERE to_agent IS NULL AND status = 'accepted';
