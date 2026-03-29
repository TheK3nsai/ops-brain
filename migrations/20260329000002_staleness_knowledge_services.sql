-- Extend staleness tracking (last_verified_at) to knowledge and services.
-- Runbooks already have this column (20260327300001).
ALTER TABLE knowledge ADD COLUMN IF NOT EXISTS last_verified_at TIMESTAMPTZ;
ALTER TABLE services ADD COLUMN IF NOT EXISTS last_verified_at TIMESTAMPTZ;
