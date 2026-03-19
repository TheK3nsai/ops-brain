use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct Vendor {
    pub id: Uuid,
    pub name: String,
    pub category: Option<String>,
    pub account_number: Option<String>,
    pub support_phone: Option<String>,
    pub support_email: Option<String>,
    pub support_portal: Option<String>,
    pub sla_summary: Option<String>,
    pub contract_end: Option<NaiveDate>,
    pub notes: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, sqlx::FromRow)]
pub struct VendorClient {
    pub vendor_id: Uuid,
    pub client_id: Uuid,
}
