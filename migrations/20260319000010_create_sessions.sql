CREATE TABLE sessions (
    id UUID PRIMARY KEY,
    machine_id TEXT NOT NULL,
    machine_hostname TEXT NOT NULL,
    started_at TIMESTAMPTZ NOT NULL DEFAULT now(),
    ended_at TIMESTAMPTZ,
    summary TEXT
);
CREATE INDEX idx_sessions_machine_id ON sessions(machine_id);
