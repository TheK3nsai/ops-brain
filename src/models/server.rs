use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Server {
    pub id: Uuid,
    pub site_id: Uuid,
    pub hostname: String,
    pub slug: String,
    pub os: Option<String>,
    pub ip_addresses: Vec<String>,
    pub ssh_alias: Option<String>,
    pub roles: Vec<String>,
    pub hardware: Option<String>,
    pub cpu: Option<String>,
    pub ram_gb: Option<i32>,
    pub storage_summary: Option<String>,
    pub is_virtual: bool,
    pub hypervisor_id: Option<Uuid>,
    pub status: String,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
