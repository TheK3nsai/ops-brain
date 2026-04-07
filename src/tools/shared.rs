use sqlx::PgPool;
use std::collections::HashMap;

use crate::embeddings::EmbeddingClient;

/// Best-effort embed and store: logs warning on failure, never blocks the caller.
pub(crate) async fn embed_and_store(
    pool: &PgPool,
    embedding_client: &Option<EmbeddingClient>,
    table: &str,
    id: uuid::Uuid,
    text: &str,
) {
    let Some(ref client) = embedding_client else {
        return;
    };
    match client.embed_text(text).await {
        Ok(embedding) => {
            let result = match table {
                "runbooks" => {
                    crate::repo::embedding_repo::store_runbook_embedding(pool, id, &embedding).await
                }
                "knowledge" => {
                    crate::repo::embedding_repo::store_knowledge_embedding(pool, id, &embedding)
                        .await
                }
                "incidents" => {
                    crate::repo::embedding_repo::store_incident_embedding(pool, id, &embedding)
                        .await
                }
                "handoffs" => {
                    crate::repo::embedding_repo::store_handoff_embedding(pool, id, &embedding).await
                }
                _ => return,
            };
            if let Err(e) = result {
                tracing::warn!("Failed to store embedding for {table}/{id}: {e}");
            }
        }
        Err(e) => {
            tracing::warn!("Failed to generate embedding for {table}/{id}: {e}");
        }
    }
}

/// Helper to get query embedding, returning None if embedding client unavailable.
pub(crate) async fn get_query_embedding(
    embedding_client: &Option<EmbeddingClient>,
    text: &str,
) -> Option<Vec<f32>> {
    let client = embedding_client.as_ref()?;
    match client.embed_text(text).await {
        Ok(emb) => Some(emb),
        Err(e) => {
            tracing::warn!("Failed to embed query: {e}");
            None
        }
    }
}

/// Build a client_id -> (slug, name) lookup from the database.
pub(crate) async fn build_client_lookup(pool: &PgPool) -> HashMap<uuid::Uuid, (String, String)> {
    match crate::repo::client_repo::list_clients(pool).await {
        Ok(clients) => clients
            .into_iter()
            .map(|c| (c.id, (c.slug, c.name)))
            .collect(),
        Err(e) => {
            tracing::warn!("Failed to build client lookup: {e}");
            HashMap::new()
        }
    }
}

/// Write audit log entries for cross-client filtering results.
pub(crate) async fn log_audit_entries(
    pool: &PgPool,
    tool_name: &str,
    requesting_client_id: Option<uuid::Uuid>,
    entity_type: &str,
    entries: &[(uuid::Uuid, Option<uuid::Uuid>, String)],
) {
    for (entity_id, owning_client_id, action) in entries {
        crate::repo::audit_log_repo::log_access(
            pool,
            tool_name,
            requesting_client_id,
            entity_type,
            *entity_id,
            *owning_client_id,
            action,
        )
        .await;
    }
}
