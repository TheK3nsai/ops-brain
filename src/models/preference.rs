use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Preference {
    pub scope: String,
    pub key: String,
    pub value: serde_json::Value,
    pub updated_at: DateTime<Utc>,
}
