-- Monitor mappings: links Uptime Kuma monitors to ops-brain servers/services
CREATE TABLE IF NOT EXISTS monitors (
    id UUID PRIMARY KEY,
    -- The monitor name as it appears in Uptime Kuma
    monitor_name TEXT NOT NULL UNIQUE,
    -- Optional link to an ops-brain server
    server_id UUID REFERENCES servers(id) ON DELETE SET NULL,
    -- Optional link to an ops-brain service
    service_id UUID REFERENCES services(id) ON DELETE SET NULL,
    -- Human-readable notes about what this monitor watches
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

-- Trigger for updated_at (reuse existing function from migration 14)
CREATE TRIGGER set_monitors_updated_at
    BEFORE UPDATE ON monitors
    FOR EACH ROW
    EXECUTE FUNCTION set_updated_at();
