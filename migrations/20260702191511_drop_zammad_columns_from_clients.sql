-- v4.0.0: Zammad retirement — drop the ticketing ID mappings from clients.
-- Zammad was fully decommissioned on 2026-07-02; the integration code
-- (src/zammad.rs, the 5 ticket tools, the briefing ticket summary) is gone,
-- so these columns no longer map to anything.
--
-- The `ticket_links` table and its FKs were already dropped in
-- 20260509122935_drop_inventory_and_incidents.sql; this finishes the cleanup on
-- the `clients` table. Forward-only: the values were meaningless Zammad row IDs.

ALTER TABLE clients
    DROP COLUMN IF EXISTS zammad_org_id,
    DROP COLUMN IF EXISTS zammad_group_id,
    DROP COLUMN IF EXISTS zammad_customer_id;
