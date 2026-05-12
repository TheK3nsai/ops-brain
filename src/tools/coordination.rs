use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{error_result, json_result, not_found};
use super::shared::{embed_and_store, get_query_embedding};
use rmcp::model::*;

// ===== HANDOFF PARAMS =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateHandoffParams {
    /// Sender agent (free-form slug, 1–80 chars, [a-zA-Z0-9._-]).
    /// Examples: "CC-Stealth", "Codex-HSR", "Gemini-Stealth".
    #[serde(alias = "from_machine")]
    pub from_agent: String,
    /// Target agent (optional — if omitted, any agent can pick it up).
    /// Same form as `from_agent`: free-form slug.
    #[serde(alias = "to_machine")]
    pub to_agent: Option<String>,
    /// Priority: low, normal, high, or critical
    pub priority: Option<String>,
    /// Category: "action" (default — persistent until completed) or "notify"
    /// (ephemeral FYI, auto-pruned from operational queries after 7 days).
    /// Use "notify" for introductions, "I just shipped X" announcements —
    /// anything the recipient doesn't need to act on.
    pub category: Option<String>,
    /// Short title for the handoff
    pub title: String,
    /// Detailed body (markdown supported)
    pub body: String,
    /// Optional structured context (JSON object)
    pub context: Option<serde_json::Value>,
    /// Session ID this handoff originates from
    pub from_session_id: Option<String>,
    /// Parent handoff ID (UUID) when this handoff is a reply to another.
    /// Enables threaded discovery via `list_replies_to_me`. The reply's
    /// `category` is preserved as written — a reply can legitimately be
    /// `action` (it requires a response) or `notify` (pure FYI).
    pub in_reply_to: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateHandoffStatusParams {
    /// Handoff ID (UUID)
    pub handoff_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompleteHandoffParams {
    /// Handoff ID (UUID)
    pub handoff_id: String,
    /// Optional commit ref (typically a git SHA) recording the work that
    /// closed this handoff. When set, lets `mark_merged` link this handoff
    /// to a merge commit later.
    pub commit_hash: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRepliesToMeParams {
    /// Your agent identifier (free-form slug). Returns handoffs whose
    /// `in_reply_to` references a handoff you originally sent.
    #[serde(alias = "my_name")]
    pub agent_name: String,
    /// Optional ISO-8601 timestamp; only replies created after this time
    /// are returned.
    pub since: Option<String>,
    /// Max results (default 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct MarkMergedParams {
    /// Handoff ID (UUID) to mark as merged.
    pub handoff_id: String,
    /// Merge commit ref (typically the merge-to-main SHA that bundled the
    /// work). Free-form — any opaque identifier works.
    pub merge_commit: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListHandoffsParams {
    /// Filter by status: pending, accepted, or completed
    pub status: Option<String>,
    /// Filter by target agent (free-form slug; exact match).
    #[serde(alias = "to_machine")]
    pub to_agent: Option<String>,
    /// Filter by source agent (free-form slug; exact match).
    #[serde(alias = "from_machine")]
    pub from_agent: Option<String>,
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

pub async fn handle_create_handoff(
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

    // Resolve optional reply-parent ID. Existence is enforced at the
    // database level by the FK; an invalid UUID returns 422-ish error
    // here, and a well-formed-but-missing UUID returns the FK violation
    // surfaced from the INSERT.
    let in_reply_to = match &p.in_reply_to {
        Some(id_str) => match uuid::Uuid::parse_str(id_str) {
            Ok(id) => Some(id),
            Err(_) => return error_result(&format!("Invalid in_reply_to UUID: {id_str}")),
        },
        None => None,
    };

    // Validate sender + target as free-form agent slugs. v2.0 dropped the
    // CC-fleet allowlist; whatever the caller says it is, it is. Stored
    // values are exact-match for `check_in` lookups, so writers and
    // readers must converge on the same string per agent.
    let from_agent = match crate::validation::validate_agent_name(&p.from_agent) {
        Ok(n) => n.to_string(),
        Err(e) => return error_result(&format!("from_agent: {e}")),
    };
    let to_agent = match p.to_agent.as_deref() {
        Some(raw) => match crate::validation::validate_agent_name(raw) {
            Ok(n) => Some(n.to_string()),
            Err(e) => return error_result(&format!("to_agent: {e}")),
        },
        None => None,
    };

    match crate::repo::handoff_repo::create_handoff(
        &brain.pool,
        from_session_id,
        &from_agent,
        to_agent.as_deref(),
        priority,
        category,
        &p.title,
        &p.body,
        p.context.as_ref(),
        in_reply_to,
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

pub async fn handle_accept_handoff(
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

pub async fn handle_complete_handoff(
    brain: &super::OpsBrain,
    p: CompleteHandoffParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };

    // Verify it exists and is not already completed or merged. Mirroring
    // the existing guard: re-completion is a no-op error, not a silent
    // overwrite of commit_hash.
    match crate::repo::handoff_repo::get_handoff(&brain.pool, id).await {
        Ok(Some(h)) if h.status == "completed" => {
            return error_result("Handoff is already completed")
        }
        Ok(Some(h)) if h.status == "merged" => return error_result("Handoff is already merged"),
        Ok(Some(_)) => {}
        Ok(None) => return not_found("Handoff", &p.handoff_id),
        Err(e) => return error_result(&format!("Database error: {e}")),
    }

    match crate::repo::handoff_repo::complete_handoff_with_commit(
        &brain.pool,
        id,
        p.commit_hash.as_deref(),
    )
    .await
    {
        Ok(handoff) => json_result(&handoff),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_list_replies_to_me(
    brain: &super::OpsBrain,
    p: ListRepliesToMeParams,
) -> CallToolResult {
    let agent = match crate::validation::validate_agent_name(&p.agent_name) {
        Ok(n) => n.to_string(),
        Err(e) => return error_result(&e),
    };
    let since = match p.since.as_deref() {
        Some(raw) => match chrono::DateTime::parse_from_rfc3339(raw) {
            Ok(ts) => Some(ts.with_timezone(&chrono::Utc)),
            Err(_) => {
                return error_result(&format!(
                    "Invalid `since` timestamp (expected RFC3339): {raw}"
                ))
            }
        },
        None => None,
    };
    let limit = p.limit.unwrap_or(20);

    match crate::repo::handoff_repo::list_replies_to_me(&brain.pool, &agent, since, limit).await {
        Ok(replies) => json_result(&replies),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_mark_merged(brain: &super::OpsBrain, p: MarkMergedParams) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };
    let merge_commit = p.merge_commit.trim();
    if merge_commit.is_empty() {
        return error_result("merge_commit cannot be empty");
    }

    // Idempotency: if already merged with the same commit, return current
    // state; if merged with a different commit, surface the conflict so
    // the caller doesn't silently overwrite.
    match crate::repo::handoff_repo::get_handoff(&brain.pool, id).await {
        Ok(Some(h)) if h.status == "merged" => {
            if h.merge_commit.as_deref() == Some(merge_commit) {
                return json_result(&h);
            }
            return error_result(&format!(
                "Handoff already merged with commit {:?}; refusing to overwrite",
                h.merge_commit.as_deref().unwrap_or("(unknown)")
            ));
        }
        Ok(Some(h)) if h.status != "completed" => {
            return error_result(&format!(
                "Handoff must be completed before it can be marked merged (current status: '{}')",
                h.status
            ));
        }
        Ok(Some(h)) if h.commit_hash.as_deref().unwrap_or("").trim().is_empty() => {
            return error_result("Handoff must have commit_hash before it can be marked merged")
        }
        Ok(Some(_)) => {}
        Ok(None) => return not_found("Handoff", &p.handoff_id),
        Err(e) => return error_result(&format!("Database error: {e}")),
    }

    match crate::repo::handoff_repo::mark_merged(&brain.pool, id, merge_commit).await {
        Ok(handoff) => json_result(&handoff),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_list_handoffs(
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

    // Validate agent-name filters as free-form slugs (no normalization).
    // Rows are stored exactly as written; callers must match the stored form.
    let to_agent_filter = match p.to_agent.as_deref() {
        Some(raw) => match crate::validation::validate_agent_name(raw) {
            Ok(n) => Some(n.to_string()),
            Err(e) => return error_result(&format!("to_agent: {e}")),
        },
        None => None,
    };
    let from_agent_filter = match p.from_agent.as_deref() {
        Some(raw) => match crate::validation::validate_agent_name(raw) {
            Ok(n) => Some(n.to_string()),
            Err(e) => return error_result(&format!("from_agent: {e}")),
        },
        None => None,
    };

    match crate::repo::handoff_repo::list_handoffs(
        &brain.pool,
        p.status.as_deref(),
        to_agent_filter.as_deref(),
        from_agent_filter.as_deref(),
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

pub async fn handle_search_handoffs(
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
                return error_result(
                    "Semantic search unavailable (embedding client not configured)",
                );
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

pub async fn handle_delete_handoff(
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_handoff_accepts_legacy_machine_aliases() {
        let params: CreateHandoffParams = serde_json::from_value(serde_json::json!({
            "from_machine": "CC-Stealth",
            "to_machine": "Codex-HSR",
            "title": "handoff",
            "body": "body"
        }))
        .unwrap();
        assert_eq!(params.from_agent, "CC-Stealth");
        assert_eq!(params.to_agent.as_deref(), Some("Codex-HSR"));
    }

    #[test]
    fn list_handoffs_accepts_legacy_machine_aliases() {
        let params: ListHandoffsParams = serde_json::from_value(serde_json::json!({
            "from_machine": "CC-Stealth",
            "to_machine": "Codex-HSR"
        }))
        .unwrap();
        assert_eq!(params.from_agent.as_deref(), Some("CC-Stealth"));
        assert_eq!(params.to_agent.as_deref(), Some("Codex-HSR"));
    }
}
