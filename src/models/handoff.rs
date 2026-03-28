use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Handoff {
    pub id: Uuid,
    pub from_session_id: Option<Uuid>,
    pub from_machine: String,
    pub to_machine: Option<String>,
    pub status: String,
    pub priority: String,
    pub title: String,
    pub body: String,
    pub context: Option<serde_json::Value>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
