-- Phase 12: Severity override on monitors + fix SMB timeout source gap

-- Fix: "SMB timeout during backup" incident has source=NULL (missed by Phase 11 migration)
UPDATE incidents SET source = 'seed' WHERE source IS NULL AND title NOT LIKE '[AUTO]%';

-- Add severity_override to monitors table.
-- When set, watchdog uses this instead of role-based severity logic.
-- Valid values: 'low', 'medium', 'high', 'critical' (NULL = use default role-based logic)
ALTER TABLE monitors ADD COLUMN IF NOT EXISTS severity_override VARCHAR(20);
