use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::helpers::{error_result, json_result, not_found_with_suggestions};
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateBriefingParams {
    /// Briefing type: "daily" or "weekly"
    pub briefing_type: String,
    /// Client slug to scope the briefing to a specific client (optional — omit for global briefing)
    pub client_slug: Option<String>,
}

/// Structured briefing data returned alongside the markdown content
#[derive(Debug, Serialize)]
pub struct BriefingData {
    pub briefing_type: String,
    pub client: Option<String>,
    pub generated_at: String,
    pub monitoring: MonitoringSummaryData,
    pub incidents: IncidentSummaryData,
    pub handoffs: HandoffSummaryData,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tickets: Option<TicketSummaryData>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub weekly_stats: Option<WeeklyStats>,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct MonitoringSummaryData {
    pub status: String,
    pub total: usize,
    pub up: usize,
    pub down: usize,
    pub pending: usize,
    pub maintenance: usize,
    pub down_monitors: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct IncidentSummaryData {
    pub open_total: usize,
    pub by_severity: std::collections::HashMap<String, usize>,
    pub oldest_open: Option<String>,
    pub watchdog_open: usize,
}

#[derive(Debug, Serialize)]
pub struct HandoffSummaryData {
    pub pending_count: usize,
    pub pending_titles: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct TicketSummaryData {
    pub open_count: usize,
    pub new_count: usize,
    pub by_priority: std::collections::HashMap<String, usize>,
}

#[derive(Debug, Serialize)]
pub struct WeeklyStats {
    pub resolved_count: usize,
    pub avg_ttr_minutes: Option<f64>,
    pub watchdog_resolved: usize,
}

// ===== HANDLERS =====

pub(crate) async fn handle_generate_briefing(
    brain: &super::OpsBrain,
    p: GenerateBriefingParams,
) -> CallToolResult {
    if let Err(msg) = crate::validation::validate_required(
        &p.briefing_type,
        "briefing_type",
        crate::validation::BRIEFING_TYPES,
    ) {
        return error_result(&msg);
    }

    let client = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    match crate::api::generate_briefing_inner(
        &brain.pool,
        &brain.kuma_configs,
        &brain.zammad_config,
        &p.briefing_type.to_lowercase(),
        client.as_ref(),
    )
    .await
    {
        Ok(output) => json_result(&output),
        Err(e) => error_result(&e),
    }
}
