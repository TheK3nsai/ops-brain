-- Phase 9: Client-scope safety — add client ownership to runbooks
ALTER TABLE runbooks ADD COLUMN client_id UUID REFERENCES clients(id) ON DELETE SET NULL;
ALTER TABLE runbooks ADD COLUMN cross_client_safe BOOLEAN NOT NULL DEFAULT false;
CREATE INDEX idx_runbooks_client_id ON runbooks(client_id);
