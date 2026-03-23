use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMonitorsParams {
    /// Filter by status: "up", "down", "pending", "maintenance" (optional)
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMonitorStatusParams {
    /// The exact monitor name as it appears in Uptime Kuma
    pub monitor_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMonitoringSummaryParams {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkMonitorParams {
    /// The exact monitor name as it appears in Uptime Kuma
    pub monitor_name: String,
    /// Server slug to link this monitor to (optional)
    pub server_slug: Option<String>,
    /// Service slug to link this monitor to (optional)
    pub service_slug: Option<String>,
    /// Notes about what this monitor watches (optional)
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkMonitorParams {
    /// The exact monitor name to remove the mapping for
    pub monitor_name: String,
}
