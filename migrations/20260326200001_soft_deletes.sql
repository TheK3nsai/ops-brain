-- Add status column to services and vendors for soft deletes.
-- Servers already have a status column.

ALTER TABLE services ADD COLUMN IF NOT EXISTS status VARCHAR(20) NOT NULL DEFAULT 'active';
ALTER TABLE vendors ADD COLUMN IF NOT EXISTS status VARCHAR(20) NOT NULL DEFAULT 'active';
