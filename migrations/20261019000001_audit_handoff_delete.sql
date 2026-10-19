-- delete_handoff stays a hard delete but leaves a tombstone in audit_log
-- (action = 'deleted'). detail holds the actor and the deleted row's summary.
ALTER TABLE audit_log ADD COLUMN detail JSONB;
