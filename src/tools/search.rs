use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, filter_cross_client, json_result, not_found};
use super::shared::{build_client_lookup, get_query_embedding, log_audit_entries};
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SemanticSearchParams {
    /// Natural language search query
    pub query: String,
    /// Tables to search (runbooks, knowledge, incidents, handoffs). Default: all.
    pub tables: Option<Vec<String>>,
    /// Max results per table (default 5)
    pub limit: Option<i64>,
    /// Scope results to a client. Cross-client runbooks/knowledge are withheld unless acknowledged.
    pub client_slug: Option<String>,
    /// Set to true to release cross-client results that were withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackfillEmbeddingsParams {
    /// Specific table to backfill (runbooks, knowledge, incidents, handoffs). Default: all.
    pub table: Option<String>,
    /// Records per batch (default 10)
    pub batch_size: Option<i64>,
}

// ===== HANDLERS =====

pub(crate) async fn handle_semantic_search(
    brain: &super::OpsBrain,
    p: SemanticSearchParams,
) -> CallToolResult {
    let limit = p.limit.unwrap_or(5);
    let tables = p.tables.unwrap_or_else(|| {
        vec![
            "runbooks".to_string(),
            "knowledge".to_string(),
            "incidents".to_string(),
            "handoffs".to_string(),
        ]
    });

    // Resolve optional client_slug for cross-client gate
    let requesting_client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found("Client", slug),
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };
    let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

    let query_embedding = get_query_embedding(&brain.embedding_client, &p.query).await;
    let emb_ref = query_embedding.as_deref();

    let mut results = serde_json::Map::new();
    let client_lookup = build_client_lookup(&brain.pool).await;
    let mut all_withheld: Vec<serde_json::Value> = Vec::new();

    // Run searches for requested tables — gate runbooks, knowledge, and incidents
    if tables.iter().any(|t| t == "runbooks") {
        match crate::repo::embedding_repo::hybrid_search_runbooks(
            &brain.pool,
            &p.query,
            emb_ref,
            limit,
        )
        .await
        {
            Ok(items) => {
                let json_items: Vec<serde_json::Value> = items
                    .iter()
                    .filter_map(|r| serde_json::to_value(r).ok())
                    .collect();
                let filtered = filter_cross_client(
                    json_items,
                    "runbook",
                    requesting_client_id,
                    acknowledge,
                    &client_lookup,
                );
                results.insert(
                    "runbooks".to_string(),
                    serde_json::to_value(&filtered.allowed).unwrap_or_default(),
                );
                all_withheld.extend(filtered.withheld_notices);
                log_audit_entries(
                    &brain.pool,
                    "semantic_search",
                    requesting_client_id,
                    "runbook",
                    &filtered.audit_entries,
                )
                .await;
            }
            Err(e) => {
                results.insert(
                    "runbooks_error".to_string(),
                    serde_json::Value::String(e.to_string()),
                );
            }
        }
    }
    if tables.iter().any(|t| t == "knowledge") {
        match crate::repo::embedding_repo::hybrid_search_knowledge(
            &brain.pool,
            &p.query,
            emb_ref,
            limit,
        )
        .await
        {
            Ok(items) => {
                let json_items: Vec<serde_json::Value> = items
                    .iter()
                    .filter_map(|k| serde_json::to_value(k).ok())
                    .collect();
                let filtered = filter_cross_client(
                    json_items,
                    "knowledge",
                    requesting_client_id,
                    acknowledge,
                    &client_lookup,
                );
                results.insert(
                    "knowledge".to_string(),
                    serde_json::to_value(&filtered.allowed).unwrap_or_default(),
                );
                all_withheld.extend(filtered.withheld_notices);
                log_audit_entries(
                    &brain.pool,
                    "semantic_search",
                    requesting_client_id,
                    "knowledge",
                    &filtered.audit_entries,
                )
                .await;
            }
            Err(e) => {
                results.insert(
                    "knowledge_error".to_string(),
                    serde_json::Value::String(e.to_string()),
                );
            }
        }
    }
    // Incidents are gated — HIPAA/IRS cross-client isolation
    if tables.iter().any(|t| t == "incidents") {
        match crate::repo::embedding_repo::hybrid_search_incidents(
            &brain.pool,
            &p.query,
            emb_ref,
            limit,
        )
        .await
        {
            Ok(items) => {
                let json_items: Vec<serde_json::Value> = items
                    .iter()
                    .filter_map(|i| serde_json::to_value(i).ok())
                    .collect();
                let filtered = filter_cross_client(
                    json_items,
                    "incident",
                    requesting_client_id,
                    acknowledge,
                    &client_lookup,
                );
                results.insert(
                    "incidents".to_string(),
                    serde_json::to_value(&filtered.allowed).unwrap_or_default(),
                );
                all_withheld.extend(filtered.withheld_notices);
                log_audit_entries(
                    &brain.pool,
                    "semantic_search",
                    requesting_client_id,
                    "incident",
                    &filtered.audit_entries,
                )
                .await;
            }
            Err(e) => {
                results.insert(
                    "incidents_error".to_string(),
                    serde_json::Value::String(e.to_string()),
                );
            }
        }
    }
    if tables.iter().any(|t| t == "handoffs") {
        match crate::repo::embedding_repo::hybrid_search_handoffs(
            &brain.pool,
            &p.query,
            emb_ref,
            limit,
        )
        .await
        {
            Ok(items) => {
                results.insert(
                    "handoffs".to_string(),
                    serde_json::to_value(&items).unwrap_or_default(),
                );
            }
            Err(e) => {
                results.insert(
                    "handoffs_error".to_string(),
                    serde_json::Value::String(e.to_string()),
                );
            }
        }
    }

    if !all_withheld.is_empty() {
        results.insert(
            "cross_client_withheld".to_string(),
            serde_json::to_value(&all_withheld).unwrap_or_default(),
        );
    }

    if query_embedding.is_none() && brain.embedding_client.is_some() {
        results.insert(
            "_note".to_string(),
            serde_json::Value::String(
                "Embedding API call failed — results are FTS-only".to_string(),
            ),
        );
    } else if brain.embedding_client.is_none() {
        results.insert(
            "_note".to_string(),
            serde_json::Value::String("OPENAI_API_KEY not set — results are FTS-only".to_string()),
        );
    }

    json_result(&serde_json::Value::Object(results))
}

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
