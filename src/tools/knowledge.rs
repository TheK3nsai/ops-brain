use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddKnowledgeParams {
    pub title: String,
    pub content: String,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub client_slug: Option<String>,
    /// Allow this entry to surface in other clients' contexts (default: false)
    pub cross_client_safe: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchKnowledgeParams {
    pub query: String,
    /// Search mode: "fts" (default), "semantic" (vector only), or "hybrid" (FTS + vector RRF)
    pub mode: Option<String>,
    /// Scope results to a client. Cross-client results are withheld unless acknowledged.
    pub client_slug: Option<String>,
    /// Set to true to release cross-client results that were withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListKnowledgeParams {
    pub category: Option<String>,
    pub client_slug: Option<String>,
}
