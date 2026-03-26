-- Runbook execution log for compliance audit trail
-- Records when a runbook was executed, by whom, and the result
CREATE TABLE IF NOT EXISTS runbook_executions (
    id UUID PRIMARY KEY,
    runbook_id UUID NOT NULL REFERENCES runbooks(id) ON DELETE CASCADE,
    executor TEXT NOT NULL,           -- who ran it (CC name, machine, or person)
    result TEXT NOT NULL DEFAULT 'success',  -- success, failure, partial, skipped
    notes TEXT,                       -- freeform notes about the execution
    duration_minutes INTEGER,         -- how long it took
    executed_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX IF NOT EXISTS idx_runbook_executions_runbook_id ON runbook_executions(runbook_id);
CREATE INDEX IF NOT EXISTS idx_runbook_executions_executed_at ON runbook_executions(executed_at DESC);
