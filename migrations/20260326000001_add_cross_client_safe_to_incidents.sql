-- Cross-client safety gate for incidents (compliance: HIPAA/IRS §7216 isolation)
ALTER TABLE incidents ADD COLUMN cross_client_safe BOOLEAN NOT NULL DEFAULT false;
