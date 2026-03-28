CREATE TABLE networks (
    id UUID PRIMARY KEY,
    site_id UUID NOT NULL REFERENCES sites(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    cidr TEXT NOT NULL,
    vlan_id INTEGER,
    gateway TEXT,
    dns_servers TEXT[] NOT NULL DEFAULT '{}',
    dhcp_server TEXT,
    purpose TEXT,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_networks_site_id ON networks(site_id);
