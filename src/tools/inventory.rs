use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetServerParams {
    /// Server slug (e.g., "hvfs0", "stealth")
    pub slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListServersParams {
    /// Filter by client slug
    pub client_slug: Option<String>,
    /// Filter by site slug
    pub site_slug: Option<String>,
    /// Filter by role (e.g., "domain-controller", "file-server")
    pub role: Option<String>,
    /// Filter by status (e.g., "active", "decommissioned")
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetServiceParams {
    /// Service slug
    pub slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListServicesParams {
    /// Filter by category
    pub category: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSiteParams {
    /// Site slug
    pub slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetClientParams {
    /// Client slug
    pub slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetNetworkParams {
    /// Filter by site slug
    pub site_slug: Option<String>,
    /// Network ID (UUID)
    pub id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetVendorParams {
    /// Vendor name
    pub name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInventoryParams {
    /// Search query
    pub query: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpsertClientParams {
    pub name: String,
    pub slug: String,
    pub notes: Option<String>,
    /// Zammad organization ID for this client
    pub zammad_org_id: Option<i32>,
    /// Zammad default ticket group ID for this client
    pub zammad_group_id: Option<i32>,
    /// Zammad default customer ID for this client
    pub zammad_customer_id: Option<i32>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpsertSiteParams {
    pub client_slug: String,
    pub name: String,
    pub slug: String,
    pub address: Option<String>,
    pub wan_provider: Option<String>,
    pub wan_ip: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpsertServerParams {
    pub site_slug: String,
    pub hostname: String,
    pub slug: String,
    pub os: Option<String>,
    pub ip_addresses: Option<Vec<String>>,
    pub ssh_alias: Option<String>,
    pub roles: Option<Vec<String>>,
    pub hardware: Option<String>,
    pub cpu: Option<String>,
    pub ram_gb: Option<i32>,
    pub storage_summary: Option<String>,
    pub is_virtual: Option<bool>,
    pub hypervisor_slug: Option<String>,
    pub status: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpsertServiceParams {
    pub name: String,
    pub slug: String,
    pub category: Option<String>,
    pub description: Option<String>,
    pub criticality: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpsertVendorParams {
    pub name: String,
    pub category: Option<String>,
    pub account_number: Option<String>,
    pub support_phone: Option<String>,
    pub support_email: Option<String>,
    pub support_portal: Option<String>,
    pub sla_summary: Option<String>,
    pub contract_end: Option<String>,
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteServerParams {
    /// Server slug to delete
    pub slug: String,
    /// Must be true to confirm deletion. If false/omitted, returns a preview of what would be affected.
    pub confirm: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteServiceParams {
    /// Service slug to delete
    pub slug: String,
    /// Must be true to confirm deletion. If false/omitted, returns a preview of what would be affected.
    pub confirm: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteVendorParams {
    /// Vendor name to delete (case-insensitive match)
    pub name: String,
    /// Must be true to confirm deletion. If false/omitted, returns a preview of what would be affected.
    pub confirm: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkServerServiceParams {
    pub server_slug: String,
    pub service_slug: String,
    pub port: Option<i32>,
    pub config_notes: Option<String>,
}
