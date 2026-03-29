use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Service {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub criticality: String,
    pub notes: Option<String>,
    pub status: String,
    pub last_verified_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServerService {
    pub server_id: Uuid,
    pub service_id: Uuid,
    pub port: Option<i32>,
    pub config_notes: Option<String>,
}

/// For queries joining service with port info
#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct ServiceWithPort {
    pub id: Uuid,
    pub name: String,
    pub slug: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub criticality: String,
    pub notes: Option<String>,
    pub port: Option<i32>,
    pub config_notes: Option<String>,
}
