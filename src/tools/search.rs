use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{error_result, json_result};
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackfillEmbeddingsParams {
    /// Specific table to backfill (runbooks, knowledge, incidents, handoffs). Default: all.
    pub table: Option<String>,
    /// Records per batch (default 10)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub batch_size: Option<i64>,
}

// ===== HANDLERS =====

pub(crate) async fn handle_backfill_embeddings(
    brain: &super::OpsBrain,
    p: BackfillEmbeddingsParams,
) -> CallToolResult {
    let Some(ref client) = brain.embedding_client else {
        return error_result("OPENAI_API_KEY not set — cannot generate embeddings");
    };

    let batch_size = p.batch_size.unwrap_or(10);
    let tables: Vec<&str> = match &p.table {
        Some(t) => vec![t.as_str()],
        None => vec!["runbooks", "knowledge", "incidents", "handoffs"],
    };

    let mut summary = serde_json::Map::new();

    for table in &tables {
        let mut processed = 0i64;
        let mut failed = 0i64;

        match *table {
            "runbooks" => {
                if let Ok(rows) = crate::repo::embedding_repo::get_runbooks_without_embeddings(
                    &brain.pool,
                    batch_size,
                )
                .await
                {
                    let texts: Vec<String> = rows
                        .iter()
                        .map(crate::embeddings::prepare_runbook_text)
                        .collect();
                    match client.embed_texts(&texts).await {
                        Ok(embeddings) => {
                            for (row, emb) in rows.iter().zip(embeddings.iter()) {
                                if crate::repo::embedding_repo::store_runbook_embedding(
                                    &brain.pool,
                                    row.id,
                                    emb,
                                )
                                .await
                                .is_ok()
                                {
                                    processed += 1;
                                } else {
                                    failed += 1;
                                }
                            }
                        }
                        Err(e) => {
                            summary.insert(
                                format!("{table}_error"),
                                serde_json::Value::String(e.to_string()),
                            );
                        }
                    }
                }
            }
            "knowledge" => {
                if let Ok(rows) = crate::repo::embedding_repo::get_knowledge_without_embeddings(
                    &brain.pool,
                    batch_size,
                )
                .await
                {
                    let texts: Vec<String> = rows
                        .iter()
                        .map(crate::embeddings::prepare_knowledge_text)
                        .collect();
                    match client.embed_texts(&texts).await {
                        Ok(embeddings) => {
                            for (row, emb) in rows.iter().zip(embeddings.iter()) {
                                if crate::repo::embedding_repo::store_knowledge_embedding(
                                    &brain.pool,
                                    row.id,
                                    emb,
                                )
                                .await
                                .is_ok()
                                {
                                    processed += 1;
                                } else {
                                    failed += 1;
                                }
                            }
                        }
                        Err(e) => {
                            summary.insert(
                                format!("{table}_error"),
                                serde_json::Value::String(e.to_string()),
                            );
                        }
                    }
                }
            }
            "incidents" => {
                if let Ok(rows) = crate::repo::embedding_repo::get_incidents_without_embeddings(
                    &brain.pool,
                    batch_size,
                )
                .await
                {
                    let texts: Vec<String> = rows
                        .iter()
                        .map(crate::embeddings::prepare_incident_text)
                        .collect();
                    match client.embed_texts(&texts).await {
                        Ok(embeddings) => {
                            for (row, emb) in rows.iter().zip(embeddings.iter()) {
                                if crate::repo::embedding_repo::store_incident_embedding(
                                    &brain.pool,
                                    row.id,
                                    emb,
                                )
                                .await
                                .is_ok()
                                {
                                    processed += 1;
                                } else {
                                    failed += 1;
                                }
                            }
                        }
                        Err(e) => {
                            summary.insert(
                                format!("{table}_error"),
                                serde_json::Value::String(e.to_string()),
                            );
                        }
                    }
                }
            }
            "handoffs" => {
                if let Ok(rows) = crate::repo::embedding_repo::get_handoffs_without_embeddings(
                    &brain.pool,
                    batch_size,
                )
                .await
                {
                    let texts: Vec<String> = rows
                        .iter()
                        .map(crate::embeddings::prepare_handoff_text)
                        .collect();
                    match client.embed_texts(&texts).await {
                        Ok(embeddings) => {
                            for (row, emb) in rows.iter().zip(embeddings.iter()) {
                                if crate::repo::embedding_repo::store_handoff_embedding(
                                    &brain.pool,
                                    row.id,
                                    emb,
                                )
                                .await
                                .is_ok()
                                {
                                    processed += 1;
                                } else {
                                    failed += 1;
                                }
                            }
                        }
                        Err(e) => {
                            summary.insert(
                                format!("{table}_error"),
                                serde_json::Value::String(e.to_string()),
                            );
                        }
                    }
                }
            }
            _ => {
                summary.insert(
                    format!("{table}_error"),
                    serde_json::Value::String("Unknown table".to_string()),
                );
                continue;
            }
        }

        summary.insert(
            format!("{table}_processed"),
            serde_json::Value::Number(processed.into()),
        );
        summary.insert(
            format!("{table}_failed"),
            serde_json::Value::Number(failed.into()),
        );
    }

    // Get remaining counts
    if let Ok(counts) = crate::repo::embedding_repo::count_missing_embeddings(&brain.pool).await {
        summary.insert(
            "remaining_runbooks".to_string(),
            serde_json::Value::Number(counts.runbooks.into()),
        );
        summary.insert(
            "remaining_knowledge".to_string(),
            serde_json::Value::Number(counts.knowledge.into()),
        );
        summary.insert(
            "remaining_incidents".to_string(),
            serde_json::Value::Number(counts.incidents.into()),
        );
        summary.insert(
            "remaining_handoffs".to_string(),
            serde_json::Value::Number(counts.handoffs.into()),
        );
    }

    json_result(&serde_json::Value::Object(summary))
}
