-- Phase 8: Scheduled briefings — store generated operational summaries
CREATE TABLE IF NOT EXISTS briefings (
    id UUID PRIMARY KEY,
    briefing_type TEXT NOT NULL CHECK (briefing_type IN ('daily', 'weekly')),
    client_id UUID REFERENCES clients(id),
    content TEXT NOT NULL,
    generated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_briefings_type ON briefings (briefing_type);
CREATE INDEX IF NOT EXISTS idx_briefings_client_id ON briefings (client_id);
CREATE INDEX IF NOT EXISTS idx_briefings_generated_at ON briefings (generated_at DESC);
