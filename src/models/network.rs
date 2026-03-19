use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Network {
    pub id: Uuid,
    pub site_id: Uuid,
    pub name: String,
    pub cidr: String,
    pub vlan_id: Option<i32>,
    pub gateway: Option<String>,
    pub dns_servers: Vec<String>,
    pub dhcp_server: Option<String>,
    pub purpose: Option<String>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}
