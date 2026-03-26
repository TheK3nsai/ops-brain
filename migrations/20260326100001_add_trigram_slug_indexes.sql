-- P2: Fuzzy slug suggestions ("did you mean?")
-- Enable pg_trgm for trigram similarity matching on slugs/names.
-- GIN indexes accelerate similarity queries on slug columns.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE INDEX IF NOT EXISTS idx_servers_slug_trgm ON servers USING gin (slug gin_trgm_ops);
CREATE INDEX IF NOT EXISTS idx_services_slug_trgm ON services USING gin (slug gin_trgm_ops);
CREATE INDEX IF NOT EXISTS idx_sites_slug_trgm ON sites USING gin (slug gin_trgm_ops);
CREATE INDEX IF NOT EXISTS idx_clients_slug_trgm ON clients USING gin (slug gin_trgm_ops);
CREATE INDEX IF NOT EXISTS idx_runbooks_slug_trgm ON runbooks USING gin (slug gin_trgm_ops);
CREATE INDEX IF NOT EXISTS idx_vendors_name_trgm ON vendors USING gin (name gin_trgm_ops);
