-- Fix embedding dimensions: 1536 (OpenAI) -> 768 (nomic-embed-text via ollama)
-- Safe: all embedding columns are NULL at this point (no data to lose)

-- Drop HNSW indexes first
DROP INDEX IF EXISTS idx_runbooks_embedding;
DROP INDEX IF EXISTS idx_knowledge_embedding;
DROP INDEX IF EXISTS idx_incidents_embedding;
DROP INDEX IF EXISTS idx_handoffs_embedding;

-- Drop and recreate columns with correct dimensions
ALTER TABLE runbooks DROP COLUMN embedding;
ALTER TABLE runbooks ADD COLUMN embedding vector(768);

ALTER TABLE knowledge DROP COLUMN embedding;
ALTER TABLE knowledge ADD COLUMN embedding vector(768);

ALTER TABLE incidents DROP COLUMN embedding;
ALTER TABLE incidents ADD COLUMN embedding vector(768);

ALTER TABLE handoffs DROP COLUMN embedding;
ALTER TABLE handoffs ADD COLUMN embedding vector(768);

-- Recreate HNSW indexes
CREATE INDEX idx_runbooks_embedding ON runbooks USING hnsw (embedding vector_cosine_ops);
CREATE INDEX idx_knowledge_embedding ON knowledge USING hnsw (embedding vector_cosine_ops);
CREATE INDEX idx_incidents_embedding ON incidents USING hnsw (embedding vector_cosine_ops);
CREATE INDEX idx_handoffs_embedding ON handoffs USING hnsw (embedding vector_cosine_ops);
