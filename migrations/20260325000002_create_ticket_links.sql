-- Phase 7: Zammad ticket links — maps Zammad ticket IDs to ops-brain entities
CREATE TABLE IF NOT EXISTS ticket_links (
    id UUID PRIMARY KEY,
    zammad_ticket_id INTEGER NOT NULL,
    incident_id UUID REFERENCES incidents(id) ON DELETE CASCADE,
    server_id UUID REFERENCES servers(id) ON DELETE SET NULL,
    service_id UUID REFERENCES services(id) ON DELETE SET NULL,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_ticket_links_zammad_id ON ticket_links(zammad_ticket_id);
CREATE INDEX IF NOT EXISTS idx_ticket_links_incident_id ON ticket_links(incident_id);
CREATE INDEX IF NOT EXISTS idx_ticket_links_server_id ON ticket_links(server_id);
CREATE INDEX IF NOT EXISTS idx_ticket_links_service_id ON ticket_links(service_id);

DROP TRIGGER IF EXISTS set_ticket_links_updated_at ON ticket_links;
CREATE TRIGGER set_ticket_links_updated_at
    BEFORE UPDATE ON ticket_links
    FOR EACH ROW
    EXECUTE FUNCTION update_updated_at();
