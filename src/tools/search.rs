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
    /// Scope results to a client. Cross-client runbooks/knowledge are withheld unless acknowledged.
    pub client_slug: Option<String>,
    /// Set to true to release cross-client results that were withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct BackfillEmbeddingsParams {
    /// Specific table to backfill (runbooks, knowledge, incidents, handoffs). Default: all.
    pub table: Option<String>,
    /// Records per batch (default 10)
    pub batch_size: Option<i64>,
}
