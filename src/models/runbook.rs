use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Runbook {
    pub id: Uuid,
    pub title: String,
    pub slug: String,
    pub category: Option<String>,
    pub content: String,
    pub version: i32,
    pub tags: Vec<String>,
    pub estimated_minutes: Option<i32>,
    pub requires_reboot: bool,
    pub notes: Option<String>,
    pub client_id: Option<Uuid>,
    pub cross_client_safe: bool,
    pub last_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
