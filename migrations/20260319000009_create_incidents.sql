CREATE TABLE incidents (
    id UUID PRIMARY KEY,
    title TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    severity TEXT NOT NULL DEFAULT 'medium',
    client_id UUID REFERENCES clients(id) ON DELETE SET NULL,
    reported_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    resolved_at TIMESTAMPTZ,
    symptoms TEXT,
    root_cause TEXT,
    resolution TEXT,
    prevention TEXT,
    time_to_resolve_minutes INTEGER,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_incidents_status ON incidents(status);
CREATE INDEX idx_incidents_severity ON incidents(severity);
CREATE INDEX idx_incidents_client_id ON incidents(client_id);

CREATE TABLE incident_servers (
    incident_id UUID NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    server_id UUID NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    PRIMARY KEY (incident_id, server_id)
);

CREATE TABLE incident_services (
    incident_id UUID NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    service_id UUID NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    PRIMARY KEY (incident_id, service_id)
);

CREATE TABLE incident_runbooks (
    incident_id UUID NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    runbook_id UUID NOT NULL REFERENCES runbooks(id) ON DELETE CASCADE,
    usage TEXT NOT NULL DEFAULT 'followed',
    PRIMARY KEY (incident_id, runbook_id)
);

CREATE TABLE incident_vendors (
    incident_id UUID NOT NULL REFERENCES incidents(id) ON DELETE CASCADE,
    vendor_id UUID NOT NULL REFERENCES vendors(id) ON DELETE CASCADE,
    PRIMARY KEY (incident_id, vendor_id)
);
