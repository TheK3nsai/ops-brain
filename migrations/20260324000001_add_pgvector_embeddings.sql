-- Phase 5: Semantic search via pgvector embeddings
CREATE EXTENSION IF NOT EXISTS vector;

-- Add embedding columns (1536 dimensions for text-embedding-3-small)
ALTER TABLE runbooks ADD COLUMN embedding vector(1536);
ALTER TABLE knowledge ADD COLUMN embedding vector(1536);
ALTER TABLE incidents ADD COLUMN embedding vector(1536);
ALTER TABLE handoffs ADD COLUMN embedding vector(1536);

-- HNSW indexes for cosine similarity search
CREATE INDEX idx_runbooks_embedding ON runbooks USING hnsw (embedding vector_cosine_ops);
CREATE INDEX idx_knowledge_embedding ON knowledge USING hnsw (embedding vector_cosine_ops);
CREATE INDEX idx_incidents_embedding ON incidents USING hnsw (embedding vector_cosine_ops);
CREATE INDEX idx_handoffs_embedding ON handoffs USING hnsw (embedding vector_cosine_ops);
