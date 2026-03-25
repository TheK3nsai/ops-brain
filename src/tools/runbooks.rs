use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetRunbookParams {
    /// Runbook slug
    pub slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListRunbooksParams {
    pub category: Option<String>,
    pub service_slug: Option<String>,
    pub server_slug: Option<String>,
    pub tag: Option<String>,
    /// Filter by owning client
    pub client_slug: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchRunbooksParams {
    /// Search query
    pub query: String,
    /// Search mode: "fts" (default), "semantic" (vector only), or "hybrid" (FTS + vector RRF)
    pub mode: Option<String>,
    /// Scope results to a client. Cross-client results are withheld unless acknowledged.
    pub client_slug: Option<String>,
    /// Set to true to release cross-client results that were withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateRunbookParams {
    pub title: String,
    pub slug: String,
    pub category: Option<String>,
    pub content: String,
    pub tags: Option<Vec<String>>,
    pub estimated_minutes: Option<i32>,
    pub requires_reboot: Option<bool>,
    pub notes: Option<String>,
    /// Assign this runbook to a client (slug). Unset = global.
    pub client_slug: Option<String>,
    /// Allow this runbook to surface in other clients' contexts (default: false)
    pub cross_client_safe: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateRunbookParams {
    pub slug: String,
    pub title: Option<String>,
    pub category: Option<String>,
    pub content: Option<String>,
    pub tags: Option<Vec<String>>,
    pub estimated_minutes: Option<i32>,
    pub requires_reboot: Option<bool>,
    pub notes: Option<String>,
    /// Allow this runbook to surface in other clients' contexts
    pub cross_client_safe: Option<bool>,
}
