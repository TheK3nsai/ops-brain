-- Drop Inventory and Incidents subsystems
-- Aligning ops-brain purely with its killer features (handoffs, knowledge, zammad ticketing).
--
-- Zammad is now the single source of truth for incidents and tickets.
-- Configuration management (e.g. Terraform, Ansible) is the source of truth for inventory.

ALTER TABLE knowledge DROP COLUMN IF EXISTS source_incident_id;

DROP TABLE IF EXISTS ticket_links CASCADE;
DROP TABLE IF EXISTS monitors CASCADE;
DROP TABLE IF EXISTS incident_servers CASCADE;
DROP TABLE IF EXISTS incident_services CASCADE;
DROP TABLE IF EXISTS incident_vendors CASCADE;
DROP TABLE IF EXISTS incident_runbooks CASCADE;
DROP TABLE IF EXISTS incidents CASCADE;

DROP TABLE IF EXISTS vendor_clients CASCADE;
DROP TABLE IF EXISTS vendors CASCADE;
DROP TABLE IF EXISTS networks CASCADE;
DROP TABLE IF EXISTS server_services CASCADE;
DROP TABLE IF EXISTS servers CASCADE;
DROP TABLE IF EXISTS services CASCADE;
DROP TABLE IF EXISTS sites CASCADE;
