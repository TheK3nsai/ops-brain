use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct CcIdentity {
    pub cc_name: String,
    pub body: String,
    pub updated_at: DateTime<Utc>,
}
