-- Unique partial index on LOWER(name) for active vendors.
-- Prevents duplicate vendor names while allowing soft-deleted vendors to have name collisions.
-- Enables ON CONFLICT upsert by name.
CREATE UNIQUE INDEX IF NOT EXISTS idx_vendors_name_unique_active
    ON vendors (LOWER(name))
    WHERE status != 'deleted';
