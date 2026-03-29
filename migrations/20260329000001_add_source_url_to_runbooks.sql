-- Add source_url to runbooks for SSOT: runbooks in ops-brain are pointers/summaries
-- referencing canonical local docs (e.g. git-tracked files on the managing CC's machine).
ALTER TABLE runbooks ADD COLUMN IF NOT EXISTS source_url TEXT;
