-- CC team self-authored identities
--
-- Each Claude Code instance writes its own confident scope via set_my_identity.
-- check_in returns this body alongside the team roster on every session start.
-- Default-empty: a CC's first session bootstraps a "write your scope" prompt.

CREATE TABLE cc_identities (
    cc_name    TEXT PRIMARY KEY,
    body       TEXT NOT NULL,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
