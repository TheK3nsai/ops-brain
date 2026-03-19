CREATE TABLE handoffs (
    id UUID PRIMARY KEY,
    from_session_id UUID REFERENCES sessions(id) ON DELETE SET NULL,
    from_machine TEXT NOT NULL,
    to_machine TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    priority TEXT NOT NULL DEFAULT 'normal',
    title TEXT NOT NULL,
    body TEXT NOT NULL,
    context JSONB,
    created_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT now()
);
CREATE INDEX idx_handoffs_status ON handoffs(status);
CREATE INDEX idx_handoffs_to_machine ON handoffs(to_machine);
