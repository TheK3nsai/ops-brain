-- Backlog improvements: runbook staleness tracking + global preferences

-- 1. Runbook staleness tracking
-- last_verified_at is set when log_runbook_execution records a successful execution.
-- NULL means "never verified" — useful for detecting stale runbooks.
ALTER TABLE runbooks ADD COLUMN IF NOT EXISTS last_verified_at TIMESTAMPTZ;

-- 2. Preferences table for global/machine/client-scoped defaults
-- Allows CCs to set defaults (e.g. compact=true) that all tools respect.
CREATE TABLE IF NOT EXISTS preferences (
    scope TEXT NOT NULL,       -- 'global', 'machine:<hostname>', 'client:<slug>'
    key TEXT NOT NULL,         -- e.g. 'compact'
    value JSONB NOT NULL,      -- e.g. true
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    PRIMARY KEY (scope, key)
);
