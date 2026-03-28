use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct AuditLog {
    pub id: Uuid,
    pub tool_name: String,
    pub requesting_client_id: Option<Uuid>,
    pub entity_type: String,
    pub entity_id: Uuid,
    pub owning_client_id: Option<Uuid>,
    pub action: String,
    pub created_at: DateTime<Utc>,
}
