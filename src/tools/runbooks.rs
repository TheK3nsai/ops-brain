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
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchRunbooksParams {
    /// Search query
    pub query: String,
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
}
