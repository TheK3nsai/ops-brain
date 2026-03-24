use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSituationalAwarenessParams {
    /// Server slug to get context for
    pub server_slug: Option<String>,
    /// Service slug to get context for
    pub service_slug: Option<String>,
    /// Client slug to get context for
    pub client_slug: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetClientOverviewParams {
    pub client_slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetServerContextParams {
    pub server_slug: String,
}

// Response structures for context tools
#[derive(Debug, Serialize)]
pub struct SituationalAwareness {
    pub server: Option<serde_json::Value>,
    pub site: Option<serde_json::Value>,
    pub client: Option<serde_json::Value>,
    pub services: Vec<serde_json::Value>,
    pub networks: Vec<serde_json::Value>,
    pub vendors: Vec<serde_json::Value>,
    pub recent_incidents: Vec<serde_json::Value>,
    pub relevant_runbooks: Vec<serde_json::Value>,
    pub pending_handoffs: Vec<serde_json::Value>,
    pub knowledge: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub monitoring: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_tickets: Vec<serde_json::Value>,
}

#[derive(Debug, Serialize)]
pub struct ClientOverview {
    pub client: serde_json::Value,
    pub sites: Vec<serde_json::Value>,
    pub servers: Vec<serde_json::Value>,
    pub services: Vec<serde_json::Value>,
    pub networks: Vec<serde_json::Value>,
    pub vendors: Vec<serde_json::Value>,
    pub recent_incidents: Vec<serde_json::Value>,
    pub pending_handoffs: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_tickets: Vec<serde_json::Value>,
}
