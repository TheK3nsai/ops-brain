CREATE TABLE server_services (
    server_id UUID NOT NULL REFERENCES servers(id) ON DELETE CASCADE,
    service_id UUID NOT NULL REFERENCES services(id) ON DELETE CASCADE,
    port INTEGER,
    config_notes TEXT,
    PRIMARY KEY (server_id, service_id)
);
