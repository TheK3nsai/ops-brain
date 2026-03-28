-- Phase 7: Add Zammad ticketing IDs to clients table
-- Maps ops-brain clients to Zammad organizations, groups, and default customers
ALTER TABLE clients
    ADD COLUMN IF NOT EXISTS zammad_org_id INTEGER,
    ADD COLUMN IF NOT EXISTS zammad_group_id INTEGER,
    ADD COLUMN IF NOT EXISTS zammad_customer_id INTEGER;
