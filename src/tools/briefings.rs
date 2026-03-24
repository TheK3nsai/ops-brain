use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateBriefingParams {
    /// Briefing type: "daily" or "weekly"
    pub briefing_type: String,
    /// Client slug to scope the briefing to a specific client (optional — omit for global briefing)
    pub client_slug: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListBriefingsParams {
    /// Filter by briefing type: "daily" or "weekly"
    pub briefing_type: Option<String>,
    /// Filter by client slug
    pub client_slug: Option<String>,
    /// Max results (default 10)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetBriefingParams {
    /// Briefing ID (UUID)
    pub id: String,
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
