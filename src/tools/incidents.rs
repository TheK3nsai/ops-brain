use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateIncidentParams {
    /// Short title describing the incident
    pub title: String,
    /// Severity: low, medium, high, or critical
    pub severity: Option<String>,
    /// Client slug this incident belongs to
    pub client_slug: Option<String>,
    /// Initial symptoms observed
    pub symptoms: Option<String>,
    /// Any initial notes
    pub notes: Option<String>,
    /// Server slugs affected by this incident
    pub server_slugs: Option<Vec<String>>,
    /// Service slugs affected by this incident
    pub service_slugs: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateIncidentParams {
    /// Incident ID (UUID)
    pub id: String,
    /// Updated title
    pub title: Option<String>,
    /// Updated status: open or resolved
    pub status: Option<String>,
    /// Updated severity: low, medium, high, or critical
    pub severity: Option<String>,
    /// Symptoms description
    pub symptoms: Option<String>,
    /// Root cause analysis
    pub root_cause: Option<String>,
    /// How it was resolved
    pub resolution: Option<String>,
    /// Steps to prevent recurrence
    pub prevention: Option<String>,
    /// Additional notes
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetIncidentParams {
    /// Incident ID (UUID)
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListIncidentsParams {
    /// Filter by client slug
    pub client_slug: Option<String>,
    /// Filter by status: open or resolved
    pub status: Option<String>,
    /// Filter by severity: low, medium, high, or critical
    pub severity: Option<String>,
    /// Max results (default 20)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchIncidentsParams {
    /// Full-text search query
    pub query: String,
    /// Search mode: "fts" (default), "semantic" (vector only), or "hybrid" (FTS + vector RRF)
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkIncidentParams {
    /// Incident ID (UUID)
    pub incident_id: String,
    /// Server slugs to link
    pub server_slugs: Option<Vec<String>>,
    /// Service slugs to link
    pub service_slugs: Option<Vec<String>>,
    /// Runbook slugs to link, with usage type
    pub runbook_links: Option<Vec<RunbookLink>>,
    /// Vendor names to link
    pub vendor_names: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunbookLink {
    /// Runbook slug
    pub slug: String,
    /// Usage: followed, not-applicable, or not-followed
    pub usage: Option<String>,
}
