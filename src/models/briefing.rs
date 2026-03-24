use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Briefing {
    pub id: Uuid,
    pub briefing_type: String,
    pub client_id: Option<Uuid>,
    pub content: String,
    pub generated_at: DateTime<Utc>,
    pub created_at: DateTime<Utc>,
}
