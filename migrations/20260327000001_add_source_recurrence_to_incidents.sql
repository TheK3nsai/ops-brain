-- Phase 11: Add source + recurrence tracking to incidents
-- Enables incident deduplication (watchdog reuses recent incidents instead of creating duplicates)
-- and source tracking for analytics (filter seed vs watchdog vs manual incidents)

-- Source field: 'watchdog', 'manual', 'seed' — identifies who created the incident
ALTER TABLE incidents ADD COLUMN IF NOT EXISTS source TEXT;

-- Recurrence count: how many times a watchdog incident has been reopened for the same monitor
ALTER TABLE incidents ADD COLUMN IF NOT EXISTS recurrence_count INTEGER NOT NULL DEFAULT 0;

-- Tag existing watchdog incidents
UPDATE incidents SET source = 'watchdog' WHERE title LIKE '[AUTO] %' AND source IS NULL;

-- Tag seed/historical incidents with realistic TTR values
-- These had TTR=0 because they were bulk-created with reported_at ≈ resolved_at
UPDATE incidents
SET source = 'seed',
    time_to_resolve_minutes = CASE
        WHEN title ILIKE '%mass account lockout%' THEN 90
        WHEN title ILIKE '%rdp session exhaustion%' THEN 30
        WHEN title ILIKE '%hvdc01%' OR title ILIKE '%dc outage%' THEN 45
        WHEN title ILIKE '%backup%repository%full%' OR title ILIKE '%backup%full%' THEN 120
        ELSE time_to_resolve_minutes
    END,
    resolved_at = CASE
        WHEN time_to_resolve_minutes = 0 AND resolved_at IS NOT NULL
            THEN reported_at + (CASE
                WHEN title ILIKE '%mass account lockout%' THEN interval '90 minutes'
                WHEN title ILIKE '%rdp session exhaustion%' THEN interval '30 minutes'
                WHEN title ILIKE '%hvdc01%' OR title ILIKE '%dc outage%' THEN interval '45 minutes'
                WHEN title ILIKE '%backup%repository%full%' OR title ILIKE '%backup%full%' THEN interval '120 minutes'
                ELSE interval '0 minutes'
            END)
        ELSE resolved_at
    END
WHERE title NOT LIKE '[AUTO] %'
  AND source IS NULL
  AND status = 'resolved'
  AND (time_to_resolve_minutes IS NULL OR time_to_resolve_minutes = 0);
