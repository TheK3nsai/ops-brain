-- Add client_slug to runbook_executions for HIPAA audit trail compliance.
-- When a cross-client runbook (e.g. AD lockout) is executed for a specific client,
-- this field captures which client context it was for.
ALTER TABLE runbook_executions ADD COLUMN IF NOT EXISTS client_slug TEXT;
