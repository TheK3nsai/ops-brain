-- Runbook-incident bi-directional linkage: track which incident
-- prompted a runbook execution. Enables "resolved using runbook X"
-- on incidents and "last used for incident Y" on runbooks.
ALTER TABLE runbook_executions ADD COLUMN IF NOT EXISTS incident_id UUID REFERENCES incidents(id);
CREATE INDEX IF NOT EXISTS idx_runbook_executions_incident_id ON runbook_executions(incident_id) WHERE incident_id IS NOT NULL;
