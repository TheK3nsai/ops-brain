use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{error_result, json_result, not_found};
use super::shared::{embed_and_store, get_query_embedding};
use rmcp::model::*;

// ===== SESSION PARAMS =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartSessionParams {
    /// Machine identifier (e.g. "stealth", "kensai-cloud")
    pub machine_id: String,
    /// Machine hostname
    pub machine_hostname: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct EndSessionParams {
    /// Session ID (UUID)
    pub session_id: String,
    /// Summary of what was accomplished in this session
    pub summary: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSessionsParams {
    /// Filter by machine ID
    pub machine_id: Option<String>,
    /// Only show active (not ended) sessions
    pub active_only: Option<bool>,
    /// Max results (default 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

// ===== HANDOFF PARAMS =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateHandoffParams {
    /// Machine this handoff is coming from
    pub from_machine: String,
    /// Target machine (optional — if omitted, any machine can pick it up)
    pub to_machine: Option<String>,
    /// Priority: low, normal, high, or critical
    pub priority: Option<String>,
    /// Short title for the handoff
    pub title: String,
    /// Detailed body (markdown supported)
    pub body: String,
    /// Optional structured context (JSON object)
    pub context: Option<serde_json::Value>,
    /// Session ID this handoff originates from
    pub from_session_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateHandoffStatusParams {
    /// Handoff ID (UUID)
    pub handoff_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListHandoffsParams {
    /// Filter by status: pending, accepted, or completed
    pub status: Option<String>,
    /// Filter by target machine
    pub to_machine: Option<String>,
    /// Filter by source machine
    pub from_machine: Option<String>,
    /// Max results (default 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchHandoffsParams {
    /// Full-text search query
    pub query: String,
    /// Search mode: "fts" (default), "semantic" (vector only), or "hybrid" (FTS + vector RRF)
    pub mode: Option<String>,
    /// Max results (default 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

// ===== SESSION HANDLERS =====

pub(crate) async fn handle_start_session(
    brain: &super::OpsBrain,
    p: StartSessionParams,
) -> CallToolResult {
    match crate::repo::session_repo::start_session(&brain.pool, &p.machine_id, &p.machine_hostname)
        .await
    {
        Ok(session) => json_result(&session),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_end_session(
    brain: &super::OpsBrain,
    p: EndSessionParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.session_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.session_id)),
    };

    match crate::repo::session_repo::end_session(&brain.pool, id, p.summary.as_deref()).await {
        Ok(session) => json_result(&session),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_list_sessions(
    brain: &super::OpsBrain,
    p: ListSessionsParams,
) -> CallToolResult {
    let limit = p.limit.unwrap_or(20);
    let active_only = p.active_only.unwrap_or(false);

    match crate::repo::session_repo::list_sessions(
        &brain.pool,
        p.machine_id.as_deref(),
        active_only,
        limit,
    )
    .await
    {
        Ok(sessions) => json_result(&sessions),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

// ===== HANDOFF HANDLERS =====

pub(crate) async fn handle_create_handoff(
    brain: &super::OpsBrain,
    p: CreateHandoffParams,
) -> CallToolResult {
    let priority = p.priority.as_deref().unwrap_or("normal");

    if let Err(msg) = crate::validation::validate_required(
        priority,
        "priority",
        crate::validation::HANDOFF_PRIORITIES,
    ) {
        return error_result(&msg);
    }

    // Resolve optional session ID
    let from_session_id = match &p.from_session_id {
        Some(id_str) => match uuid::Uuid::parse_str(id_str) {
            Ok(id) => Some(id),
            Err(_) => return error_result(&format!("Invalid session UUID: {id_str}")),
        },
        None => None,
    };

    match crate::repo::handoff_repo::create_handoff(
        &brain.pool,
        from_session_id,
        &p.from_machine,
        p.to_machine.as_deref(),
        priority,
        &p.title,
        &p.body,
        p.context.as_ref(),
    )
    .await
    {
        Ok(handoff) => {
            let text = crate::embeddings::prepare_handoff_text(&handoff);
            embed_and_store(
                &brain.pool,
                &brain.embedding_client,
                "handoffs",
                handoff.id,
                &text,
            )
            .await;
            json_result(&handoff)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_accept_handoff(
    brain: &super::OpsBrain,
    p: UpdateHandoffStatusParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };

    // Verify it's pending
    match crate::repo::handoff_repo::get_handoff(&brain.pool, id).await {
        Ok(Some(h)) if h.status == "pending" => {}
        Ok(Some(h)) => {
            return error_result(&format!("Handoff is already '{}', cannot accept", h.status))
        }
        Ok(None) => return not_found("Handoff", &p.handoff_id),
        Err(e) => return error_result(&format!("Database error: {e}")),
    }

    match crate::repo::handoff_repo::update_handoff_status(&brain.pool, id, "accepted").await {
        Ok(handoff) => json_result(&handoff),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_complete_handoff(
    brain: &super::OpsBrain,
    p: UpdateHandoffStatusParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };

    // Verify it exists and is not already completed
    match crate::repo::handoff_repo::get_handoff(&brain.pool, id).await {
        Ok(Some(h)) if h.status == "completed" => {
            return error_result("Handoff is already completed")
        }
        Ok(Some(_)) => {}
        Ok(None) => return not_found("Handoff", &p.handoff_id),
        Err(e) => return error_result(&format!("Database error: {e}")),
    }

    match crate::repo::handoff_repo::update_handoff_status(&brain.pool, id, "completed").await {
        Ok(handoff) => json_result(&handoff),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_list_handoffs(
    brain: &super::OpsBrain,
    p: ListHandoffsParams,
) -> CallToolResult {
    let limit = p.limit.unwrap_or(20);

    if let Err(msg) = crate::validation::validate_option(
        p.status.as_deref(),
        "status",
        crate::validation::HANDOFF_STATUSES,
    ) {
        return error_result(&msg);
    }

    match crate::repo::handoff_repo::list_handoffs(
        &brain.pool,
        p.status.as_deref(),
        p.to_machine.as_deref(),
        p.from_machine.as_deref(),
        limit,
    )
    .await
    {
        Ok(handoffs) => json_result(&handoffs),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_search_handoffs(
    brain: &super::OpsBrain,
    p: SearchHandoffsParams,
) -> CallToolResult {
    let mode = p.mode.as_deref().unwrap_or("fts");
    if let Err(msg) =
        crate::validation::validate_required(mode, "mode", crate::validation::SEARCH_MODES)
    {
        return error_result(&msg);
    }
    let limit = p.limit.unwrap_or(20);
    let result = match mode {
        "semantic" => {
            let Some(emb) = get_query_embedding(&brain.embedding_client, &p.query).await else {
                return error_result("Semantic search unavailable (OPENAI_API_KEY not set)");
            };
            crate::repo::embedding_repo::vector_search_handoffs(&brain.pool, &emb, limit).await
        }
        "hybrid" => {
            let emb = get_query_embedding(&brain.embedding_client, &p.query).await;
            crate::repo::embedding_repo::hybrid_search_handoffs(
                &brain.pool,
                &p.query,
                emb.as_deref(),
                limit,
            )
            .await
        }
        _ => crate::repo::handoff_repo::search_handoffs(&brain.pool, &p.query, limit).await,
    };
    match result {
        Ok(handoffs) => json_result(&handoffs),
        Err(e) => error_result(&format!("Search error: {e}")),
    }
}
