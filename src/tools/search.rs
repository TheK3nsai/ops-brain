use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SemanticSearchParams {
    /// Natural language search query
    pub query: String,
    /// Tables to search (runbooks, knowledge, incidents, handoffs). Default: all.
    pub tables: Option<Vec<String>>,
    /// Max results per table (default 5)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackfillEmbeddingsParams {
    /// Specific table to backfill (runbooks, knowledge, incidents, handoffs). Default: all.
    pub table: Option<String>,
    /// Records per batch (default 10)
    pub batch_size: Option<i64>,
}
