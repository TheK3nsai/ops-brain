//! REST API endpoints for ops-brain.
//!
//! These provide simple HTTP access to ops-brain data without requiring
//! MCP protocol negotiation. Protected by the same bearer auth middleware.

use axum::{
    extract::{Query, State},
    http::StatusCode,
    Extension, Json,
};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;

use crate::auth::CallerClass;
use crate::tools::briefings;
use crate::validation::{
    validate_agent_name, validate_option, HANDOFF_CATEGORIES, HANDOFF_PRIORITIES,
};

/// Shared state for REST API handlers.
#[derive(Clone)]
pub struct ApiState {
    pub pool: PgPool,
}

// ===== Machine ingestion: POST /api/handoff =====

/// Top-level keys of the machine-context convention v1. Unknown keys are
/// warned about, never rejected — the contract is documented in
/// `docs/machine-callers.md` and versioned by the `v` field.
const CONTEXT_V1_KEYS: &[&str] = &[
    "v",
    "source",
    "check",
    "verdict",
    "observed_at",
    "evidence_ref",
    "metrics",
];
const CONTEXT_VERDICTS: &[&str] = &["PASS", "WARN", "FAIL", "UNKNOWN"];

const MAX_TITLE_CHARS: usize = 500;
const MAX_BODY_CHARS: usize = 100_000;
const MAX_DEDUPE_KEY_CHARS: usize = 200;
/// Context is stored uncompressed and serialized into every MCP reader
/// response untruncated (unlike body) — keep it a small structured payload,
/// not an evidence dump. Evidence belongs behind `evidence_ref`.
const MAX_CONTEXT_CHARS: usize = 8_192;

#[derive(Debug, Deserialize)]
pub struct CreateHandoffRequest {
    /// Required for main-bearer callers. Machine callers get theirs from the
    /// token binding — supplying a *different* value is a 400.
    pub from_agent: Option<String>,
    /// Required for machine callers (and must be within the token's agent
    /// allowlist). Optional for main-bearer callers (open handoff).
    pub to_agent: Option<String>,
    pub priority: Option<String>,
    pub category: Option<String>,
    pub title: String,
    pub body: String,
    pub context: Option<serde_json::Value>,
    /// Idempotency key for recurring producers. While a handoff with this
    /// key is open, repeat POSTs no-op into a repeat_count bump.
    pub dedupe_key: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct CreateHandoffResponse {
    pub id: String,
    pub status: String,
    /// True when the POST was suppressed into an existing open handoff
    /// holding the same dedupe_key.
    pub deduplicated: bool,
    /// Suppressed-duplicate count on the returned row (0 = filed once).
    pub repeat_count: i32,
    /// Lenient-validation notes (unknown context keys etc.). Never fatal.
    pub warnings: Vec<String>,
}

fn bad_request(msg: impl Into<String>) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, msg.into())
}

fn validate_dedupe_key(key: &str) -> Result<(), String> {
    if key.is_empty() || key.len() > MAX_DEDUPE_KEY_CHARS {
        return Err(format!(
            "dedupe_key must be 1–{MAX_DEDUPE_KEY_CHARS} chars, got {}",
            key.len()
        ));
    }
    if !key
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/'))
    {
        return Err(format!(
            "dedupe_key '{key}' contains invalid characters (allowed: a-zA-Z0-9 . - _ /)"
        ));
    }
    Ok(())
}

/// Lenient context-convention check: reject only structural problems
/// (non-object), warn on everything else. The producer sees the warnings in
/// the response; nothing is dropped.
fn check_context(context: &serde_json::Value) -> Result<Vec<String>, String> {
    let Some(obj) = context.as_object() else {
        return Err("context must be a JSON object when present".into());
    };
    let serialized_len = context.to_string().len();
    if serialized_len > MAX_CONTEXT_CHARS {
        return Err(format!(
            "context too large ({serialized_len} chars serialized, max {MAX_CONTEXT_CHARS}) — \
             point at evidence via evidence_ref instead of inlining it"
        ));
    }
    let mut warnings = Vec::new();
    for key in obj.keys() {
        if !CONTEXT_V1_KEYS.contains(&key.as_str()) {
            warnings.push(format!(
                "context key '{key}' is not part of convention v1 (kept as-is; \
                 see docs/machine-callers.md)"
            ));
        }
    }
    if let Some(v) = obj.get("verdict").and_then(|v| v.as_str()) {
        if !CONTEXT_VERDICTS.contains(&v) {
            warnings.push(format!(
                "context.verdict '{v}' is not one of {}",
                CONTEXT_VERDICTS.join("|")
            ));
        }
    }
    Ok(warnings)
}

/// POST /api/handoff — file a handoff from a non-interactive caller.
///
/// `origin` is stamped `machine` unconditionally: this is the machine
/// ingestion path regardless of which credential reached it. Interactive
/// agents file through the MCP `create_handoff` tool (`origin = agent`).
pub async fn create_handoff(
    State(state): State<Arc<ApiState>>,
    Extension(caller): Extension<CallerClass>,
    Json(req): Json<CreateHandoffRequest>,
) -> Result<(StatusCode, Json<CreateHandoffResponse>), (StatusCode, String)> {
    // Resolve identity from the caller class.
    let from_agent = match &caller {
        CallerClass::Machine(token) => {
            if let Some(claimed) = req.from_agent.as_deref() {
                if !claimed.eq_ignore_ascii_case(&token.from_agent) {
                    return Err(bad_request(format!(
                        "from_agent '{claimed}' does not match this token's binding — omit the \
                         field; the token IS the identity"
                    )));
                }
            }
            token.from_agent.clone()
        }
        CallerClass::Full => {
            let claimed = req
                .from_agent
                .as_deref()
                .ok_or_else(|| bad_request("from_agent is required"))?;
            validate_agent_name(claimed)
                .map_err(bad_request)?
                .to_string()
        }
    };

    // Routing scope.
    let to_agent = match (&caller, req.to_agent.as_deref()) {
        (CallerClass::Machine(_), None) => {
            return Err(bad_request(
                "to_agent is required for machine-filed handoffs (no open filings)",
            ));
        }
        (CallerClass::Machine(token), Some(to)) => {
            let to = validate_agent_name(to).map_err(bad_request)?;
            if !token.allows_agent(to) {
                return Err((
                    StatusCode::FORBIDDEN,
                    format!("this token may not file handoffs to '{to}'"),
                ));
            }
            to.to_string()
        }
        (CallerClass::Full, Some(to)) => validate_agent_name(to).map_err(bad_request)?.to_string(),
        (CallerClass::Full, None) => {
            return Err(bad_request(
                "to_agent is required on the REST ingestion path",
            ));
        }
    };

    validate_option(req.priority.as_deref(), "priority", HANDOFF_PRIORITIES)
        .map_err(bad_request)?;
    validate_option(req.category.as_deref(), "category", HANDOFF_CATEGORIES)
        .map_err(bad_request)?;
    let priority = req.priority.as_deref().unwrap_or("normal").to_lowercase();
    let category = req.category.as_deref().unwrap_or("action").to_lowercase();

    if req.title.trim().is_empty() || req.title.len() > MAX_TITLE_CHARS {
        return Err(bad_request(format!(
            "title must be 1–{MAX_TITLE_CHARS} chars"
        )));
    }
    if req.body.trim().is_empty() || req.body.len() > MAX_BODY_CHARS {
        return Err(bad_request(format!(
            "body must be 1–{MAX_BODY_CHARS} chars"
        )));
    }
    if let Some(key) = req.dedupe_key.as_deref() {
        validate_dedupe_key(key).map_err(bad_request)?;
    }

    let warnings = match req.context.as_ref() {
        Some(ctx) => check_context(ctx).map_err(bad_request)?,
        None => Vec::new(),
    };

    let handoff = crate::repo::handoff_repo::create_machine_handoff(
        &state.pool,
        &from_agent,
        &to_agent,
        &priority,
        &category,
        &req.title,
        &req.body,
        req.context.as_ref(),
        req.dedupe_key.as_deref(),
    )
    .await
    .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {e}")))?;

    // The upsert returns the pre-existing row on dedupe suppression; a fresh
    // insert always carries repeat_count 0 and the id we just minted — so a
    // bumped repeat_count is the reliable dedupe signal.
    let deduplicated = handoff.repeat_count > 0;
    tracing::info!(
        from_agent = %from_agent,
        to_agent = %to_agent,
        id = %handoff.id,
        deduplicated,
        repeat_count = handoff.repeat_count,
        "machine-filed handoff"
    );

    let status = if deduplicated {
        StatusCode::OK
    } else {
        StatusCode::CREATED
    };
    Ok((
        status,
        Json(CreateHandoffResponse {
            id: handoff.id.to_string(),
            status: handoff.status,
            deduplicated,
            repeat_count: handoff.repeat_count,
            warnings,
        }),
    ))
}

// ===== Wake poll: GET /api/pending =====

#[derive(Debug, Deserialize)]
pub struct PendingQuery {
    /// Agent whose open action queue to poll.
    pub agent: String,
    /// Optional ISO-8601 cursor: only items with `updated_at` after this
    /// instant (dedupe bumps re-surface a still-firing monitor).
    pub since: Option<String>,
    pub limit: Option<i64>,
}

/// Trimmed item shape for the poll — deliberately body-free so a 5-minute
/// cadence stays a few hundred bytes. Agents fetch full bodies over MCP
/// once awake.
#[derive(Debug, serde::Serialize)]
pub struct PendingItem {
    pub id: String,
    pub title: String,
    pub status: String,
    pub priority: String,
    pub category: String,
    pub origin: String,
    pub from_agent: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dedupe_key: Option<String>,
    pub repeat_count: i32,
    pub created_at: chrono::DateTime<chrono::Utc>,
    pub updated_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, serde::Serialize)]
pub struct PendingResponse {
    pub count: usize,
    pub items: Vec<PendingItem>,
}

/// GET /api/pending?agent=X&since=ISO — open action handoffs for an agent,
/// cheap enough for dumb local schedulers to poll on a short interval.
pub async fn list_pending(
    State(state): State<Arc<ApiState>>,
    Extension(caller): Extension<CallerClass>,
    Query(q): Query<PendingQuery>,
) -> Result<Json<PendingResponse>, (StatusCode, String)> {
    let agent = validate_agent_name(&q.agent).map_err(bad_request)?;

    if let CallerClass::Machine(token) = &caller {
        if !token.allows_agent(agent) {
            return Err((
                StatusCode::FORBIDDEN,
                format!("this token may not poll the queue of '{agent}'"),
            ));
        }
    }

    let since = match q.since.as_deref() {
        Some(raw) => Some(
            chrono::DateTime::parse_from_rfc3339(raw)
                .map(|dt| dt.with_timezone(&chrono::Utc))
                .map_err(|e| bad_request(format!("invalid since timestamp '{raw}': {e}")))?,
        ),
        None => None,
    };
    let limit = q.limit.unwrap_or(50).clamp(1, 200);

    let handoffs =
        crate::repo::handoff_repo::list_pending_for_agent(&state.pool, agent, since, limit)
            .await
            .map_err(|e| (StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {e}")))?;

    let items: Vec<PendingItem> = handoffs
        .into_iter()
        .map(|h| PendingItem {
            id: h.id.to_string(),
            title: h.title,
            status: h.status,
            priority: h.priority,
            category: h.category,
            origin: h.origin,
            from_agent: h.from_agent,
            dedupe_key: h.dedupe_key,
            repeat_count: h.repeat_count,
            created_at: h.created_at,
            updated_at: h.updated_at,
        })
        .collect();

    Ok(Json(PendingResponse {
        count: items.len(),
        items,
    }))
}

#[derive(Debug, Deserialize)]
pub struct GenerateBriefingRequest {
    /// "daily" or "weekly"
    #[serde(rename = "type")]
    pub briefing_type: String,
    /// Optional client slug to scope the briefing
    pub client_slug: Option<String>,
}

/// POST /api/briefing — generate and return an operational briefing.
pub async fn generate_briefing(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<GenerateBriefingRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let briefing_type = req.briefing_type.to_lowercase();
    if !["daily", "weekly"].contains(&briefing_type.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Invalid type: '{}'. Use 'daily' or 'weekly'.",
                req.briefing_type
            ),
        ));
    }

    let client = match &req.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&state.pool, slug).await {
            Ok(Some(c)) => Some(c),
            Ok(None) => return Err((StatusCode::NOT_FOUND, format!("Client not found: {slug}"))),
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {e}"))),
        },
        None => None,
    };

    match generate_briefing_inner(&state.pool, &briefing_type, client.as_ref()).await {
        Ok(data) => Ok(Json(data)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

/// Core briefing generation logic shared between the MCP tool and the REST API.
pub async fn generate_briefing_inner(
    pool: &PgPool,
    briefing_type: &str,
    client: Option<&crate::models::client::Client>,
) -> Result<serde_json::Value, String> {
    let is_weekly = briefing_type == "weekly";
    let client_id = client.map(|c| c.id);
    let client_name = client.map(|c| c.name.as_str()).unwrap_or("All Clients");

    // ── Pending handoffs ──
    // Briefings show actionable work only — notify-class FYIs are not "pending"
    // in any meaningful sense.
    let open_handoffs =
        crate::repo::handoff_repo::list_open_handoffs(pool, None, None, None, false, 20)
            .await
            .unwrap_or_default();

    let pending_titles: Vec<String> = open_handoffs
        .iter()
        .filter(|h| h.status == "pending")
        .map(|h| h.title.clone())
        .collect();
    let accepted_titles: Vec<String> = open_handoffs
        .iter()
        .filter(|h| h.status == "accepted")
        .map(|h| h.title.clone())
        .collect();

    let handoff_data = briefings::HandoffSummaryData {
        open_count: open_handoffs.len(),
        pending_count: pending_titles.len(),
        accepted_count: accepted_titles.len(),
        pending_titles,
        accepted_titles,
    };

    // ── Build markdown ──
    let now = chrono::Utc::now();
    let md = build_markdown(is_weekly, client_name, &now, &handoff_data);

    // Store
    let briefing = crate::repo::briefing_repo::insert_briefing(pool, briefing_type, client_id, &md)
        .await
        .map_err(|e| format!("Failed to store briefing: {e}"))?;

    let result = briefings::BriefingData {
        briefing_type: briefing_type.to_string(),
        client: client.map(|c| c.slug.clone()),
        generated_at: now.format("%Y-%m-%d %H:%M UTC").to_string(),
        handoffs: handoff_data,
        content: md,
    };

    let mut output = serde_json::to_value(&result).unwrap_or_default();
    output["briefing_id"] = serde_json::Value::String(briefing.id.to_string());
    Ok(output)
}

fn build_markdown(
    is_weekly: bool,
    client_name: &str,
    now: &chrono::DateTime<chrono::Utc>,
    handoffs: &briefings::HandoffSummaryData,
) -> String {
    let mut md = String::new();

    md.push_str(&format!(
        "# {} Operational Briefing — {}\n",
        if is_weekly { "Weekly" } else { "Daily" },
        client_name
    ));
    md.push_str(&format!(
        "*Generated: {}*\n\n",
        now.format("%Y-%m-%d %H:%M UTC")
    ));

    // Handoffs
    md.push_str("## Handoffs\n\n");
    if handoffs.open_count == 0 {
        md.push_str("No open handoffs.\n\n");
    } else {
        md.push_str(&format!(
            "**{} open handoff(s)** ({} pending, {} accepted)\n\n",
            handoffs.open_count, handoffs.pending_count, handoffs.accepted_count
        ));
        if !handoffs.pending_titles.is_empty() {
            md.push_str("Pending:\n");
            for title in &handoffs.pending_titles {
                md.push_str(&format!("- {title}\n"));
            }
        }
        if !handoffs.accepted_titles.is_empty() {
            md.push_str("Accepted:\n");
            for title in &handoffs.accepted_titles {
                md.push_str(&format!("- {title}\n"));
            }
        }
        md.push('\n');
    }

    md
}
