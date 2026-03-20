use schemars::JsonSchema;
use serde::Deserialize;

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
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchHandoffsParams {
    /// Full-text search query
    pub query: String,
}
