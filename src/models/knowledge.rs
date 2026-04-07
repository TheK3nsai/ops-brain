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
    /// Which CC authored this entry (CC-Cloud, CC-Stealth, CC-HSR, CC-CPA).
    /// NULL for rows created before v1.6. Required on new entries; immutable
    /// once set via the tool surface.
    pub author_cc: Option<String>,
    /// Incident that produced this knowledge entry, if any. NULL if
    /// standalone or pre-dates v1.6. FK with ON DELETE SET NULL.
    pub source_incident_id: Option<Uuid>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
