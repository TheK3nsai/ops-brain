use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Monitor {
    pub id: Uuid,
    pub monitor_name: String,
    pub server_id: Option<Uuid>,
    pub service_id: Option<Uuid>,
    pub notes: Option<String>,
    /// Override watchdog severity for this monitor. Values: low, medium, high, critical.
    /// NULL = use default role-based logic from server roles.
    pub severity_override: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
