-- Optional webhook delivery is passive notification, never task scheduling.
-- Callback signing keys are secrets: protect database access and backups.
CREATE TABLE mcp_event_subscriptions (
    id TEXT PRIMARY KEY,
    owner_key TEXT NOT NULL,
    oauth_principal JSONB,
    agent_name TEXT NOT NULL,
    restricted BOOLEAN NOT NULL,
    callback_url TEXT NOT NULL,
    signing_secret TEXT NOT NULL,
    previous_secret TEXT,
    rotation_until TIMESTAMPTZ,
    verified_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    expires_at TIMESTAMPTZ NOT NULL,
    active BOOLEAN NOT NULL DEFAULT true,
    generation BIGINT NOT NULL DEFAULT 1,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX mcp_event_subscriptions_owner ON mcp_event_subscriptions(owner_key);
CREATE TABLE mcp_event_outbox (
    subscription_id TEXT NOT NULL REFERENCES mcp_event_subscriptions(id) ON DELETE CASCADE,
    handoff_id UUID NOT NULL REFERENCES handoffs(id) ON DELETE CASCADE,
    generation BIGINT NOT NULL,
    occurred_at TIMESTAMPTZ NOT NULL,
    attempts INTEGER NOT NULL DEFAULT 0,
    next_attempt_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    finished_at TIMESTAMPTZ,
    PRIMARY KEY (subscription_id, handoff_id)
);
CREATE INDEX mcp_event_outbox_pending ON mcp_event_outbox(next_attempt_at) WHERE finished_at IS NULL;
CREATE FUNCTION enqueue_handoff_created_event() RETURNS trigger LANGUAGE plpgsql AS $$
BEGIN
    -- Explicit inbound queues only. No broadcast or arbitrary fleet subscriptions.
    INSERT INTO mcp_event_outbox(subscription_id, handoff_id, generation, occurred_at)
    SELECT id, NEW.id, generation, NEW.created_at
    FROM mcp_event_subscriptions
    WHERE active AND expires_at > now() AND lower(agent_name) = lower(NEW.to_agent);
    RETURN NEW;
END;
$$;
CREATE TRIGGER handoff_created_event AFTER INSERT ON handoffs
    FOR EACH ROW EXECUTE FUNCTION enqueue_handoff_created_event();
