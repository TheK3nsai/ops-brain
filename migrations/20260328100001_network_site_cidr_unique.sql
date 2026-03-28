-- Add unique constraint on (site_id, cidr) to support upsert_network ON CONFLICT
CREATE UNIQUE INDEX IF NOT EXISTS idx_networks_site_cidr ON networks (site_id, cidr);
