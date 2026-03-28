CREATE TABLE servers (
    id UUID PRIMARY KEY,
    site_id UUID NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    hostname TEXT NOT NULL,
    slug TEXT NOT NULL UNIQUE,
    os TEXT,
    ip_addresses TEXT[] NOT NULL DEFAULT '{}',
    ssh_alias TEXT,
    roles TEXT[] NOT NULL DEFAULT '{}',
    hardware TEXT,
    cpu TEXT,
    ram_gb INTEGER,
    storage_summary TEXT,
    is_virtual BOOLEAN NOT NULL DEFAULT false,
    hypervisor_id UUID REFERENCES servers(id) ON DELETE SET NULL,
    status TEXT NOT NULL DEFAULT 'active',
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_servers_slug ON servers(slug);
CREATE INDEX idx_servers_site_id ON servers(site_id);
CREATE INDEX idx_servers_status ON servers(status);
CREATE INDEX idx_servers_roles ON servers USING GIN(roles);
