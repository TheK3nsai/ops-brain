-- Chronic flapper suppression: per-monitor threshold for auto-downgrade/suppress.
-- NULL means use the global default (OPS_BRAIN_WATCHDOG_FLAP_THRESHOLD env var).
-- When recurrence_count >= flap_threshold, watchdog auto-downgrades severity to 'low'.
-- When recurrence_count >= 2 * flap_threshold, watchdog auto-resolves immediately.
ALTER TABLE monitors ADD COLUMN IF NOT EXISTS flap_threshold INTEGER;
