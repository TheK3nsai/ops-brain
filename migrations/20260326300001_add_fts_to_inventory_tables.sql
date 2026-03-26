-- Add FTS search_vector to vendors, clients, sites, and networks
-- so search_inventory can cover all entity types.

-- Vendors FTS (name + category + notes)
ALTER TABLE vendors ADD COLUMN IF NOT EXISTS search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(name, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(category, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(notes, '')), 'C')
    ) STORED;
CREATE INDEX IF NOT EXISTS idx_vendors_fts ON vendors USING GIN(search_vector);

-- Clients FTS (name + slug + notes)
ALTER TABLE clients ADD COLUMN IF NOT EXISTS search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(name, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(slug, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(notes, '')), 'B')
    ) STORED;
CREATE INDEX IF NOT EXISTS idx_clients_fts ON clients USING GIN(search_vector);

-- Sites FTS (name + slug + address + notes)
ALTER TABLE sites ADD COLUMN IF NOT EXISTS search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(name, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(slug, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(address, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(notes, '')), 'C')
    ) STORED;
CREATE INDEX IF NOT EXISTS idx_sites_fts ON sites USING GIN(search_vector);

-- Networks FTS (name + cidr + purpose + notes)
ALTER TABLE networks ADD COLUMN IF NOT EXISTS search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(name, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(cidr, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(purpose, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(notes, '')), 'C')
    ) STORED;
CREATE INDEX IF NOT EXISTS idx_networks_fts ON networks USING GIN(search_vector);
