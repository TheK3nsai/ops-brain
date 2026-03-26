use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Incident {
    pub id: Uuid,
    pub title: String,
    pub status: String,
    pub severity: String,
    pub client_id: Option<Uuid>,
    pub reported_at: DateTime<Utc>,
    pub resolved_at: Option<DateTime<Utc>>,
    pub symptoms: Option<String>,
    pub root_cause: Option<String>,
    pub resolution: Option<String>,
    pub prevention: Option<String>,
    pub time_to_resolve_minutes: Option<i32>,
    pub notes: Option<String>,
    pub cross_client_safe: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
