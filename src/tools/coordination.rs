use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{error_result, json_result, not_found};
use super::shared::{embed_and_store, get_query_embedding};
use rmcp::model::*;

// ===== HANDOFF PARAMS =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateHandoffParams {
    /// Machine this handoff is coming from
    pub from_machine: String,
    /// Target machine (optional — if omitted, any machine can pick it up)
    pub to_machine: Option<String>,
    /// Priority: low, normal, high, or critical
    pub priority: Option<String>,
    /// Category: "action" (default — persistent until completed) or "notify"
    /// (ephemeral FYI, auto-pruned from operational queries after 7 days).
    /// Use "notify" for introductions, watchdog drops, "I just shipped X"
    /// announcements — anything the recipient doesn't need to act on.
    pub category: Option<String>,
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
    /// Filter by category: "action" or "notify". Overrides include_notify
    /// when set. Omit to use the default action-only view.
    pub category: Option<String>,
    /// Include notify-class handoffs alongside action ones (default: false).
    /// Ignored when `category` is set explicitly.
    pub include_notify: Option<bool>,
    /// Max results (default 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
    /// Truncate body to 200 chars (default: true). Set false for full bodies.
    pub compact: Option<bool>,
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

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteHandoffParams {
    /// Handoff ID (UUID)
    pub handoff_id: String,
}

// ===== HANDOFF HANDLERS =====

pub(crate) async fn handle_create_handoff(
    brain: &super::OpsBrain,
    p: CreateHandoffParams,
) -> CallToolResult {
    let priority = p.priority.as_deref().unwrap_or("normal");
    let category = p.category.as_deref().unwrap_or("action");

    if let Err(msg) = crate::validation::validate_required(
        priority,
        "priority",
        crate::validation::HANDOFF_PRIORITIES,
    ) {
        return error_result(&msg);
    }
    if let Err(msg) = crate::validation::validate_required(
        category,
        "category",
        crate::validation::HANDOFF_CATEGORIES,
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
        category,
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
    let compact = p.compact.unwrap_or(true);
    let include_notify = p.include_notify.unwrap_or(false);

    if let Err(msg) = crate::validation::validate_option(
        p.status.as_deref(),
        "status",
        crate::validation::HANDOFF_STATUSES,
    ) {
        return error_result(&msg);
    }
    if let Err(msg) = crate::validation::validate_option(
        p.category.as_deref(),
        "category",
        crate::validation::HANDOFF_CATEGORIES,
    ) {
        return error_result(&msg);
    }

    match crate::repo::handoff_repo::list_handoffs(
        &brain.pool,
        p.status.as_deref(),
        p.to_machine.as_deref(),
        p.from_machine.as_deref(),
        p.category.as_deref(),
        include_notify,
        limit,
    )
    .await
    {
        Ok(handoffs) => {
            if compact {
                let compacted: Vec<serde_json::Value> = handoffs
                    .iter()
                    .filter_map(|h| {
                        let mut val = serde_json::to_value(h).ok()?;
                        if let Some(obj) = val.as_object_mut() {
                            if let Some(serde_json::Value::String(body)) = obj.get("body") {
                                let truncated = if body.len() > 200 {
                                    format!("{}...", &body[..body.floor_char_boundary(200)])
                                } else {
                                    body.clone()
                                };
                                obj.insert(
                                    "body".to_string(),
                                    serde_json::Value::String(truncated),
                                );
                            }
                        }
                        Some(val)
                    })
                    .collect();
                json_result(&compacted)
            } else {
                json_result(&handoffs)
            }
        }
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

pub(crate) async fn handle_delete_handoff(
    brain: &super::OpsBrain,
    p: DeleteHandoffParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };

    match crate::repo::handoff_repo::delete_handoff(&brain.pool, id).await {
        Ok(true) => json_result(&serde_json::json!({"deleted": true, "id": p.handoff_id})),
        Ok(false) => not_found("Handoff", &p.handoff_id),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}
