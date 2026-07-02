//! REST API endpoints for ops-brain.
//!
//! These provide simple HTTP access to ops-brain data without requiring
//! MCP protocol negotiation. Protected by the same bearer auth middleware.

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;

use crate::tools::briefings;

/// Shared state for REST API handlers.
#[derive(Clone)]
pub struct ApiState {
    pub pool: PgPool,
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
