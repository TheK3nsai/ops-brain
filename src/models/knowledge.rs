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
    pub last_verified_at: Option<DateTime<Utc>>,
    /// Agent that authored this entry (free-form slug, e.g. "CC-Stealth",
    /// "codex-hsr"). NULL for rows created before v1.6. Required on new
    /// entries; immutable once set via the tool surface.
    pub author: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
