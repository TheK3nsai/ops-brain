use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, filter_cross_client, json_result, not_found};
use super::shared::{build_client_lookup, embed_and_store, get_query_embedding, log_audit_entries};
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddKnowledgeParams {
    pub title: String,
    pub content: String,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub client_slug: Option<String>,
    /// Allow this entry to surface in other clients' contexts (default: false)
    pub cross_client_safe: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchKnowledgeParams {
    pub query: String,
    /// Search mode: "fts" (default), "semantic" (vector only), or "hybrid" (FTS + vector RRF)
    pub mode: Option<String>,
    /// Scope results to a client. Cross-client results are withheld unless acknowledged.
    pub client_slug: Option<String>,
    /// Set to true to release cross-client results that were withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateKnowledgeParams {
    /// Knowledge entry ID (UUID)
    pub id: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Allow this entry to surface in other clients' contexts
    pub cross_client_safe: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteKnowledgeParams {
    /// Knowledge entry ID (UUID)
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListKnowledgeParams {
    pub category: Option<String>,
    pub client_slug: Option<String>,
}

// ===== HANDLERS =====

pub(crate) async fn handle_add_knowledge(
    brain: &super::OpsBrain,
    p: AddKnowledgeParams,
) -> CallToolResult {
    let tags = p.tags.unwrap_or_default();
    let cross_client_safe = p.cross_client_safe.unwrap_or(false);

    // Resolve optional client_slug
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found("Client", slug),
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    match crate::repo::knowledge_repo::add_knowledge(
        &brain.pool,
        &p.title,
        &p.content,
        p.category.as_deref(),
        &tags,
        client_id,
        cross_client_safe,
    )
    .await
    {
        Ok(entry) => {
            let text = crate::embeddings::prepare_knowledge_text(&entry);
            embed_and_store(
                &brain.pool,
                &brain.embedding_client,
                "knowledge",
                entry.id,
                &text,
            )
            .await;
            json_result(&entry)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_update_knowledge(
    brain: &super::OpsBrain,
    p: UpdateKnowledgeParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.id)),
    };

    // Verify entry exists
    match crate::repo::knowledge_repo::get_knowledge(&brain.pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("Knowledge", &p.id),
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    match crate::repo::knowledge_repo::update_knowledge(
        &brain.pool,
        id,
        p.title.as_deref(),
        p.content.as_deref(),
        p.category.as_deref(),
        p.tags.as_deref(),
        p.cross_client_safe,
    )
    .await
    {
        Ok(updated) => {
            let text = crate::embeddings::prepare_knowledge_text(&updated);
            embed_and_store(
                &brain.pool,
                &brain.embedding_client,
                "knowledge",
                updated.id,
                &text,
            )
            .await;
            json_result(&updated)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_delete_knowledge(
    brain: &super::OpsBrain,
    p: DeleteKnowledgeParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.id)),
    };

    match crate::repo::knowledge_repo::delete_knowledge(&brain.pool, id).await {
        Ok(true) => json_result(&serde_json::json!({"deleted": true, "id": p.id})),
        Ok(false) => not_found("Knowledge", &p.id),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_search_knowledge(
    brain: &super::OpsBrain,
    p: SearchKnowledgeParams,
) -> CallToolResult {
    let mode = p.mode.as_deref().unwrap_or("fts");
    if let Err(msg) =
        crate::validation::validate_required(mode, "mode", crate::validation::SEARCH_MODES)
    {
        return error_result(&msg);
    }

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

    let result = match mode {
        "semantic" => {
            let Some(emb) = get_query_embedding(&brain.embedding_client, &p.query).await else {
                return error_result("Semantic search unavailable (OPENAI_API_KEY not set)");
            };
            crate::repo::embedding_repo::vector_search_knowledge(&brain.pool, &emb, 20).await
        }
        "hybrid" => {
            let emb = get_query_embedding(&brain.embedding_client, &p.query).await;
            crate::repo::embedding_repo::hybrid_search_knowledge(
                &brain.pool,
                &p.query,
                emb.as_deref(),
                20,
            )
            .await
        }
        _ => crate::repo::knowledge_repo::search_knowledge(&brain.pool, &p.query).await,
    };
    match result {
        Ok(entries) => {
            let items: Vec<serde_json::Value> = entries
                .iter()
                .filter_map(|k| serde_json::to_value(k).ok())
                .collect();
            let client_lookup = build_client_lookup(&brain.pool).await;
            let filtered = filter_cross_client(
                items,
                "knowledge",
                requesting_client_id,
                acknowledge,
                &client_lookup,
            );

            log_audit_entries(
                &brain.pool,
                "search_knowledge",
                requesting_client_id,
                "knowledge",
                &filtered.audit_entries,
            )
            .await;

            let mut response = serde_json::json!({ "knowledge": filtered.allowed });
            if !filtered.withheld_notices.is_empty() {
                response["cross_client_withheld"] = serde_json::json!(filtered.withheld_notices);
            }
            json_result(&response)
        }
        Err(e) => error_result(&format!("Search error: {e}")),
    }
}

pub(crate) async fn handle_list_knowledge(
    brain: &super::OpsBrain,
    p: ListKnowledgeParams,
) -> CallToolResult {
    // Resolve optional client_slug
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found("Client", slug),
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    match crate::repo::knowledge_repo::list_knowledge(&brain.pool, p.category.as_deref(), client_id)
        .await
    {
        Ok(entries) => json_result(&entries),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}
