CREATE TABLE runbooks (
    id UUID PRIMARY KEY,
    title TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    category TEXT,
    content TEXT NOT NULL,
    version INTEGER NOT NULL DEFAULT 1,
    tags TEXT[] NOT NULL DEFAULT '{}',
    estimated_minutes INTEGER,
    requires_reboot BOOLEAN NOT NULL DEFAULT false,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_runbooks_slug ON runbooks(slug);
CREATE INDEX idx_runbooks_category ON runbooks(category);
CREATE INDEX idx_runbooks_tags ON runbooks USING GIN(tags);

CREATE TABLE runbook_services (
    runbook_id UUID NOT NULL REFERENCES runbooks(id) ON DELETE CASCADE,
    service_id UUID NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    PRIMARY KEY (runbook_id, service_id)
);

CREATE TABLE runbook_servers (
    runbook_id UUID NOT NULL REFERENCES runbooks(id) ON DELETE CASCADE,
    server_id UUID NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    PRIMARY KEY (runbook_id, server_id)
);
