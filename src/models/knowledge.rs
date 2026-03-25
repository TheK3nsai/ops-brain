use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Knowledge {
    pub id: Uuid,
    pub title: String,
    pub content: String,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub client_id: Option<Uuid>,
    pub cross_client_safe: bool,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
