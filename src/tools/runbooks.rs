use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, filter_cross_client, json_result, not_found_with_suggestions};
use super::shared::{build_client_lookup, embed_and_store, get_query_embedding, log_audit_entries};
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRunbookParams {
    /// Runbook slug
    pub slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRunbooksParams {
    pub category: Option<String>,
    pub service_slug: Option<String>,
    pub server_slug: Option<String>,
    pub tag: Option<String>,
    /// Filter by owning client
    pub client_slug: Option<String>,
    /// Max results (default 50)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchRunbooksParams {
    /// Search query
    pub query: String,
    /// Search mode: "fts" (default), "semantic" (vector only), or "hybrid" (FTS + vector RRF)
    pub mode: Option<String>,
    /// Scope results to a client. Cross-client results are withheld unless acknowledged.
    pub client_slug: Option<String>,
    /// Set to true to release cross-client results that were withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
    /// Max results (default 20)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateRunbookParams {
    pub title: String,
    pub slug: String,
    pub category: Option<String>,
    pub content: String,
    pub tags: Option<Vec<String>>,
    pub estimated_minutes: Option<i32>,
    pub requires_reboot: Option<bool>,
    pub notes: Option<String>,
    /// Assign this runbook to a client (slug). Unset = global.
    pub client_slug: Option<String>,
    /// Allow this runbook to surface in other clients' contexts (default: false)
    pub cross_client_safe: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRunbookParams {
    pub slug: String,
    pub title: Option<String>,
    pub category: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub estimated_minutes: Option<i32>,
    pub requires_reboot: Option<bool>,
    pub notes: Option<String>,
    /// Allow this runbook to surface in other clients' contexts
    pub cross_client_safe: Option<bool>,
}

// ===== HANDLERS =====

pub(crate) async fn handle_get_runbook(
    brain: &super::OpsBrain,
    p: GetRunbookParams,
) -> CallToolResult {
    match crate::repo::runbook_repo::get_runbook_by_slug(&brain.pool, &p.slug).await {
        Ok(Some(runbook)) => json_result(&runbook),
        Ok(None) => not_found_with_suggestions(&brain.pool, "Runbook", &p.slug).await,
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_list_runbooks(
    brain: &super::OpsBrain,
    p: ListRunbooksParams,
) -> CallToolResult {
    // Resolve optional client_slug
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    // Resolve optional service_slug
    let service_id = match &p.service_slug {
        Some(slug) => {
            match crate::repo::service_repo::get_service_by_slug(&brain.pool, slug).await {
                Ok(Some(s)) => Some(s.id),
                Ok(None) => return not_found_with_suggestions(&brain.pool, "Service", slug).await,
                Err(e) => return error_result(&format!("Database error: {e}")),
            }
        }
        None => None,
    };

    // Resolve optional server_slug
    let server_id = match &p.server_slug {
        Some(slug) => match crate::repo::server_repo::get_server_by_slug(&brain.pool, slug).await {
            Ok(Some(s)) => Some(s.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Server", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    let limit = p.limit.unwrap_or(50);
    match crate::repo::runbook_repo::list_runbooks(
        &brain.pool,
        p.category.as_deref(),
        service_id,
        server_id,
        p.tag.as_deref(),
        client_id,
        limit,
    )
    .await
    {
        Ok(runbooks) => json_result(&runbooks),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_search_runbooks(
    brain: &super::OpsBrain,
    p: SearchRunbooksParams,
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
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };
    let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

    let limit = p.limit.unwrap_or(20);
    let result = match mode {
        "semantic" => {
            let Some(emb) = get_query_embedding(&brain.embedding_client, &p.query).await else {
                return error_result("Semantic search unavailable (OPENAI_API_KEY not set)");
            };
            crate::repo::embedding_repo::vector_search_runbooks(&brain.pool, &emb, limit).await
        }
        "hybrid" => {
            let emb = get_query_embedding(&brain.embedding_client, &p.query).await;
            crate::repo::embedding_repo::hybrid_search_runbooks(
                &brain.pool,
                &p.query,
                emb.as_deref(),
                limit,
            )
            .await
        }
        _ => crate::repo::search_repo::search_runbooks(&brain.pool, &p.query, limit).await,
    };
    match result {
        Ok(runbooks) => {
            let items: Vec<serde_json::Value> = runbooks
                .iter()
                .filter_map(|r| serde_json::to_value(r).ok())
                .collect();
            let client_lookup = build_client_lookup(&brain.pool).await;
            let filtered = filter_cross_client(
                items,
                "runbook",
                requesting_client_id,
                acknowledge,
                &client_lookup,
            );

            // Log audit entries
            log_audit_entries(
                &brain.pool,
                "search_runbooks",
                requesting_client_id,
                "runbook",
                &filtered.audit_entries,
            )
            .await;

            let mut response = serde_json::json!({ "runbooks": filtered.allowed });
            if !filtered.withheld_notices.is_empty() {
                response["cross_client_withheld"] = serde_json::json!(filtered.withheld_notices);
            }
            json_result(&response)
        }
        Err(e) => error_result(&format!("Search error: {e}")),
    }
}

pub(crate) async fn handle_create_runbook(
    brain: &super::OpsBrain,
    p: CreateRunbookParams,
) -> CallToolResult {
    let tags = p.tags.unwrap_or_default();
    let requires_reboot = p.requires_reboot.unwrap_or(false);
    let cross_client_safe = p.cross_client_safe.unwrap_or(false);

    // Resolve optional client_slug
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    match crate::repo::runbook_repo::create_runbook(
        &brain.pool,
        &p.title,
        &p.slug,
        p.category.as_deref(),
        &p.content,
        &tags,
        p.estimated_minutes,
        requires_reboot,
        p.notes.as_deref(),
        client_id,
        cross_client_safe,
    )
    .await
    {
        Ok(runbook) => {
            let text = crate::embeddings::prepare_runbook_text(&runbook);
            embed_and_store(
                &brain.pool,
                &brain.embedding_client,
                "runbooks",
                runbook.id,
                &text,
            )
            .await;
            json_result(&runbook)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_update_runbook(
    brain: &super::OpsBrain,
    p: UpdateRunbookParams,
) -> CallToolResult {
    let runbook = match crate::repo::runbook_repo::get_runbook_by_slug(&brain.pool, &p.slug).await {
        Ok(Some(r)) => r,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Runbook", &p.slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    // Wrap estimated_minutes in Option<Option<i32>> for COALESCE
    let estimated_minutes: Option<Option<i32>> = p.estimated_minutes.map(Some);

    match crate::repo::runbook_repo::update_runbook(
        &brain.pool,
        runbook.id,
        p.title.as_deref(),
        p.category.as_deref(),
        p.content.as_deref(),
        p.tags.as_deref(),
        estimated_minutes,
        p.requires_reboot,
        p.notes.as_deref(),
        p.cross_client_safe,
    )
    .await
    {
        Ok(updated) => {
            let text = crate::embeddings::prepare_runbook_text(&updated);
            embed_and_store(
                &brain.pool,
                &brain.embedding_client,
                "runbooks",
                updated.id,
                &text,
            )
            .await;
            json_result(&updated)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}
