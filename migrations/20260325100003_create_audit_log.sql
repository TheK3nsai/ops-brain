-- Phase 9: Client-scope safety — audit trail for cross-client data access
CREATE TABLE audit_log (
    id UUID PRIMARY KEY,
    tool_name TEXT NOT NULL,
    requesting_client_id UUID REFERENCES clients(id),
    entity_type TEXT NOT NULL,
    entity_id UUID NOT NULL,
    owning_client_id UUID,
    action TEXT NOT NULL,  -- 'withheld' or 'released'
    created_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_audit_log_created_at ON audit_log(created_at);
CREATE INDEX idx_audit_log_entity ON audit_log(entity_type, entity_id);
