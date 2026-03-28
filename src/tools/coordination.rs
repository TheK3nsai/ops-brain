use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{error_result, json_result, not_found};
use super::shared::{embed_and_store, get_query_embedding, resolve_compact};
use rmcp::model::*;

// ===== SESSION PARAMS =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartSessionParams {
    /// Machine identifier (e.g. "dev-laptop", "prod-server")
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

// ===== CATCHUP PARAMS =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetCatchupParams {
    /// ISO 8601 timestamp — show changes since this time (e.g. "2026-03-25T00:00:00Z")
    pub since: String,
    /// Filter to a specific machine (optional)
    pub machine: Option<String>,
    /// Max results per category (default 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
    /// Summary fields only, excludes completed handoffs (default: true). Set false for full bodies.
    pub compact: Option<bool>,
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
    let compact = resolve_compact(&brain.pool, p.compact, true).await;

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

// ===== CATCHUP HANDLER =====

pub(crate) async fn handle_get_catchup(
    brain: &super::OpsBrain,
    p: GetCatchupParams,
) -> CallToolResult {
    let since = match chrono::DateTime::parse_from_rfc3339(&p.since) {
        Ok(dt) => dt.with_timezone(&chrono::Utc),
        Err(_) => {
            return error_result(&format!(
                "Invalid timestamp '{}'. Use ISO 8601 format (e.g. 2026-03-25T00:00:00Z)",
                p.since
            ))
        }
    };
    let limit = p.limit.unwrap_or(20);
    let compact = resolve_compact(&brain.pool, p.compact, true).await;

    let mut results = serde_json::Map::new();

    // New/updated handoffs since timestamp
    // In compact mode, exclude completed handoffs (noise for orientation)
    let handoff_query = if compact {
        "SELECT * FROM handoffs WHERE updated_at > $1 AND status != 'completed' ORDER BY updated_at DESC LIMIT $2"
    } else {
        "SELECT * FROM handoffs WHERE updated_at > $1 ORDER BY updated_at DESC LIMIT $2"
    };
    let handoffs = sqlx::query_as::<_, crate::models::handoff::Handoff>(handoff_query)
        .bind(since)
        .bind(limit)
        .fetch_all(&brain.pool)
        .await;

    match handoffs {
        Ok(items) => {
            // Optionally filter by machine
            let filtered: Vec<_> = if let Some(ref machine) = p.machine {
                items
                    .into_iter()
                    .filter(|h| {
                        h.to_machine.as_deref() == Some(machine.as_str())
                            || h.from_machine == *machine
                    })
                    .collect()
            } else {
                items
            };
            if compact {
                let json_items: Vec<serde_json::Value> = filtered
                    .iter()
                    .filter_map(|h| serde_json::to_value(h).ok())
                    .collect();
                let compacted = super::helpers::compact_vec(&json_items, "handoff");
                results.insert(
                    "handoffs".to_string(),
                    serde_json::json!({
                        "count": compacted.len(),
                        "items": compacted,
                    }),
                );
            } else {
                results.insert(
                    "handoffs".to_string(),
                    serde_json::json!({
                        "count": filtered.len(),
                        "items": filtered,
                    }),
                );
            }
        }
        Err(e) => {
            results.insert(
                "handoffs_error".to_string(),
                serde_json::Value::String(e.to_string()),
            );
        }
    }

    // New/updated incidents since timestamp
    let incidents = sqlx::query_as::<_, crate::models::incident::Incident>(
        "SELECT * FROM incidents WHERE updated_at > $1 ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(since)
    .bind(limit)
    .fetch_all(&brain.pool)
    .await;

    match incidents {
        Ok(items) => {
            if compact {
                let json_items: Vec<serde_json::Value> = items
                    .iter()
                    .filter_map(|i| serde_json::to_value(i).ok())
                    .collect();
                let compacted = super::helpers::compact_vec(&json_items, "incident");
                results.insert(
                    "incidents".to_string(),
                    serde_json::json!({
                        "count": compacted.len(),
                        "items": compacted,
                    }),
                );
            } else {
                results.insert(
                    "incidents".to_string(),
                    serde_json::json!({
                        "count": items.len(),
                        "items": items,
                    }),
                );
            }
        }
        Err(e) => {
            results.insert(
                "incidents_error".to_string(),
                serde_json::Value::String(e.to_string()),
            );
        }
    }

    // New/updated knowledge since timestamp
    let knowledge = sqlx::query_as::<_, crate::models::knowledge::Knowledge>(
        "SELECT * FROM knowledge WHERE updated_at > $1 ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(since)
    .bind(limit)
    .fetch_all(&brain.pool)
    .await;

    match knowledge {
        Ok(items) => {
            if compact {
                let json_items: Vec<serde_json::Value> = items
                    .iter()
                    .filter_map(|k| serde_json::to_value(k).ok())
                    .collect();
                let compacted = super::helpers::compact_vec(&json_items, "knowledge");
                results.insert(
                    "knowledge".to_string(),
                    serde_json::json!({
                        "count": compacted.len(),
                        "items": compacted,
                    }),
                );
            } else {
                results.insert(
                    "knowledge".to_string(),
                    serde_json::json!({
                        "count": items.len(),
                        "items": items,
                    }),
                );
            }
        }
        Err(e) => {
            results.insert(
                "knowledge_error".to_string(),
                serde_json::Value::String(e.to_string()),
            );
        }
    }

    // New/updated runbooks since timestamp
    let runbooks = sqlx::query_as::<_, crate::models::runbook::Runbook>(
        "SELECT * FROM runbooks WHERE updated_at > $1 ORDER BY updated_at DESC LIMIT $2",
    )
    .bind(since)
    .bind(limit)
    .fetch_all(&brain.pool)
    .await;

    match runbooks {
        Ok(items) => {
            if compact {
                let json_items: Vec<serde_json::Value> = items
                    .iter()
                    .filter_map(|r| serde_json::to_value(r).ok())
                    .collect();
                let compacted = super::helpers::compact_vec(&json_items, "runbook");
                results.insert(
                    "runbooks".to_string(),
                    serde_json::json!({
                        "count": compacted.len(),
                        "items": compacted,
                    }),
                );
            } else {
                results.insert(
                    "runbooks".to_string(),
                    serde_json::json!({
                        "count": items.len(),
                        "items": items,
                    }),
                );
            }
        }
        Err(e) => {
            results.insert(
                "runbooks_error".to_string(),
                serde_json::Value::String(e.to_string()),
            );
        }
    }

    // Stale runbooks: not verified in 30+ days (or never verified)
    match crate::repo::runbook_repo::list_stale_runbooks(&brain.pool, 30, 10).await {
        Ok(stale) if !stale.is_empty() => {
            let items: Vec<serde_json::Value> = stale
                .iter()
                .map(|r| {
                    serde_json::json!({
                        "slug": r.slug,
                        "title": r.title,
                        "category": r.category,
                        "last_verified_at": r.last_verified_at,
                        "updated_at": r.updated_at,
                    })
                })
                .collect();
            results.insert(
                "stale_runbooks".to_string(),
                serde_json::json!({
                    "count": items.len(),
                    "threshold_days": 30,
                    "items": items,
                    "_tip": "Runbooks not verified in 30+ days. Use log_runbook_execution with result='success' to mark as verified.",
                }),
            );
        }
        Ok(_) => {} // no stale runbooks — don't clutter the response
        Err(e) => {
            results.insert(
                "stale_runbooks_error".to_string(),
                serde_json::Value::String(e.to_string()),
            );
        }
    }

    results.insert("since".to_string(), serde_json::Value::String(p.since));
    if compact {
        results.insert(
            "_tip".to_string(),
            serde_json::Value::String(
                "Compact mode: summary fields only, completed handoffs excluded. \
                 Use compact=false for full bodies, or drill down with get_incident/get_runbook/get_ticket."
                    .to_string(),
            ),
        );
    }

    json_result(&serde_json::Value::Object(results))
}

// ===== PREFERENCE PARAMS + HANDLER =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetPreferenceParams {
    /// Preference key (e.g. "compact")
    pub key: String,
    /// Preference value (e.g. true, false, "hybrid")
    pub value: serde_json::Value,
    /// Scope: "global" (default), "machine:<hostname>", or "client:<slug>"
    pub scope: Option<String>,
}

pub(crate) async fn handle_set_preference(
    brain: &super::OpsBrain,
    p: SetPreferenceParams,
) -> CallToolResult {
    let scope = p.scope.as_deref().unwrap_or("global");

    let valid_keys = ["compact"];
    if !valid_keys.contains(&p.key.as_str()) {
        return error_result(&format!(
            "Unknown preference key '{}'. Valid keys: {}",
            p.key,
            valid_keys.join(", ")
        ));
    }

    match crate::repo::preference_repo::set_preference(&brain.pool, scope, &p.key, &p.value).await {
        Ok(pref) => json_result(&serde_json::json!({
            "scope": pref.scope,
            "key": pref.key,
            "value": pref.value,
            "updated_at": pref.updated_at,
            "_tip": "This preference will be used as the default when the parameter is not explicitly set in tool calls.",
        })),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}
