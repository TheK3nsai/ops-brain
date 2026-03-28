-- Phase 9: Client-scope safety — add cross-client safety flag to knowledge
ALTER TABLE knowledge ADD COLUMN cross_client_safe BOOLEAN NOT NULL DEFAULT false;
