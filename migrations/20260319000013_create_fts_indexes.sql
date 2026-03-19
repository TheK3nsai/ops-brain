-- Runbooks FTS
ALTER TABLE runbooks ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(content, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(notes, '')), 'C')
    ) STORED;
CREATE INDEX idx_runbooks_fts ON runbooks USING GIN(search_vector);

-- Incidents FTS
ALTER TABLE incidents ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(symptoms, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(root_cause, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(resolution, '')), 'B') ||
        setweight(to_tsvector('english', coalesce(notes, '')), 'C')
    ) STORED;
CREATE INDEX idx_incidents_fts ON incidents USING GIN(search_vector);

-- Knowledge FTS
ALTER TABLE knowledge ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(content, '')), 'B')
    ) STORED;
CREATE INDEX idx_knowledge_fts ON knowledge USING GIN(search_vector);

-- Handoffs FTS
ALTER TABLE handoffs ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(title, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(body, '')), 'B')
    ) STORED;
CREATE INDEX idx_handoffs_fts ON handoffs USING GIN(search_vector);

-- Servers FTS (hostname + notes)
ALTER TABLE servers ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(hostname, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(notes, '')), 'B')
    ) STORED;
CREATE INDEX idx_servers_fts ON servers USING GIN(search_vector);

-- Services FTS
ALTER TABLE services ADD COLUMN search_vector tsvector
    GENERATED ALWAYS AS (
        setweight(to_tsvector('english', coalesce(name, '')), 'A') ||
        setweight(to_tsvector('english', coalesce(description, '')), 'B')
    ) STORED;
CREATE INDEX idx_services_fts ON services USING GIN(search_vector);
