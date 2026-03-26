use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct RunbookExecution {
    pub id: Uuid,
    pub runbook_id: Uuid,
    pub executor: String,
    pub result: String,
    pub notes: Option<String>,
    pub duration_minutes: Option<i32>,
    pub executed_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
