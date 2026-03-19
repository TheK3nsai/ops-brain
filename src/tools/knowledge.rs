use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddKnowledgeParams {
    pub title: String,
    pub content: String,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub client_slug: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchKnowledgeParams {
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListKnowledgeParams {
    pub category: Option<String>,
    pub client_slug: Option<String>,
}
