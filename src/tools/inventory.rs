use rmcp::model::*;
use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{
    error_result, json_result, not_found, not_found_vendor_with_suggestions,
    not_found_with_suggestions,
};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetServerParams {
    /// Server slug (e.g., "web-server-01", "db-primary")
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
    /// Max results (default 50)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
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
    /// Max results (default 50)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
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
    /// Vendor name (case-insensitive). Ignored if id is provided.
    pub name: Option<String>,
    /// Vendor ID (UUID). Takes precedence over name for disambiguation.
    pub id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchInventoryParams {
    /// Search query
    pub query: String,
    /// Max results per entity type (default 10)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
    /// Client slug — when set, runbooks/knowledge/incidents from other clients are gated
    pub client_slug: Option<String>,
    /// Release cross-client results withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
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
    /// Set to true to mark this service as verified (confirms notes/config are still accurate).
    pub verified: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpsertVendorParams {
    /// Vendor name (required for create, used for matching on upsert)
    pub name: String,
    /// Vendor ID (UUID). When provided, updates the specific vendor by ID instead of name-based upsert.
    pub id: Option<String>,
    /// Client slug to link this vendor to (optional). Creates vendor_clients association.
    pub client_slug: Option<String>,
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
pub struct UpsertNetworkParams {
    /// Site slug to link this network to
    pub site_slug: String,
    /// Network name
    pub name: String,
    /// Network CIDR (e.g. "192.168.0.0/24") — unique per site
    pub cidr: String,
    /// Optional VLAN ID
    pub vlan_id: Option<i32>,
    /// Optional gateway IP
    pub gateway: Option<String>,
    /// DNS servers (array of IPs)
    pub dns_servers: Option<Vec<String>>,
    /// Optional DHCP server IP
    pub dhcp_server: Option<String>,
    /// Network purpose (e.g. "Office LAN", "DMZ", "Management")
    pub purpose: Option<String>,
    /// Optional notes
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteServerParams {
    /// Server slug to delete
    pub slug: String,
    /// True to confirm. Omit for impact preview.
    pub confirm: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteServiceParams {
    /// Service slug to delete
    pub slug: String,
    /// True to confirm. Omit for impact preview.
    pub confirm: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteVendorParams {
    /// Vendor name to delete (case-insensitive match). Ignored if id is provided.
    pub name: Option<String>,
    /// Vendor ID (UUID). Takes precedence over name for disambiguation.
    pub id: Option<String>,
    /// True to confirm. Omit for impact preview.
    pub confirm: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListVendorsParams {
    /// Filter by vendor category (e.g., "isp", "saas", "hardware")
    pub category: Option<String>,
    /// Filter by client slug (show only vendors linked to this client)
    pub client_slug: Option<String>,
    /// Max results (default 50)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListClientsParams {
    /// Max results (default 50)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListSitesParams {
    /// Filter by client slug
    pub client_slug: Option<String>,
    /// Max results (default 50)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListNetworksParams {
    /// Filter by site slug
    pub site_slug: Option<String>,
    /// Max results (default 50)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkServerServiceParams {
    pub server_slug: String,
    pub service_slug: String,
    pub port: Option<i32>,
    pub config_notes: Option<String>,
}

// ---------------------------------------------------------------------------
// Handler functions
// ---------------------------------------------------------------------------

pub(crate) async fn handle_get_server(
    brain: &super::OpsBrain,
    p: GetServerParams,
) -> CallToolResult {
    let server = match crate::repo::server_repo::get_server_by_slug(&brain.pool, &p.slug).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Server", &p.slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    let services = crate::repo::service_repo::get_services_for_server(&brain.pool, server.id)
        .await
        .unwrap_or_default();
    let site = crate::repo::site_repo::get_site(&brain.pool, server.site_id)
        .await
        .ok()
        .flatten();
    let networks = crate::repo::network_repo::list_networks(&brain.pool, Some(server.site_id))
        .await
        .unwrap_or_default();

    let result = serde_json::json!({
        "server": server,
        "services": services,
        "site": site,
        "networks": networks,
    });
    json_result(&result)
}

pub(crate) async fn handle_list_servers(
    brain: &super::OpsBrain,
    p: ListServersParams,
) -> CallToolResult {
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };
    let site_id = match &p.site_slug {
        Some(slug) => match crate::repo::site_repo::get_site_by_slug(&brain.pool, slug).await {
            Ok(Some(s)) => Some(s.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Site", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };
    let limit = p.limit.unwrap_or(50);
    match crate::repo::server_repo::list_servers(
        &brain.pool,
        client_id,
        site_id,
        p.role.as_deref(),
        p.status.as_deref(),
        limit,
    )
    .await
    {
        Ok(servers) => json_result(&servers),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_get_service(
    brain: &super::OpsBrain,
    p: GetServiceParams,
) -> CallToolResult {
    let service = match crate::repo::service_repo::get_service_by_slug(&brain.pool, &p.slug).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Service", &p.slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    let servers = crate::repo::service_repo::get_servers_for_service(&brain.pool, service.id)
        .await
        .unwrap_or_default();
    let result = serde_json::json!({
        "service": service,
        "servers": servers,
    });
    json_result(&result)
}

pub(crate) async fn handle_list_services(
    brain: &super::OpsBrain,
    p: ListServicesParams,
) -> CallToolResult {
    let limit = p.limit.unwrap_or(50);
    match crate::repo::service_repo::list_services(&brain.pool, p.category.as_deref(), limit).await
    {
        Ok(services) => json_result(&services),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_get_site(brain: &super::OpsBrain, p: GetSiteParams) -> CallToolResult {
    let site = match crate::repo::site_repo::get_site_by_slug(&brain.pool, &p.slug).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Site", &p.slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    let servers =
        crate::repo::server_repo::list_servers(&brain.pool, None, Some(site.id), None, None, 200)
            .await
            .unwrap_or_default();
    let networks = crate::repo::network_repo::list_networks(&brain.pool, Some(site.id))
        .await
        .unwrap_or_default();
    let result = serde_json::json!({
        "site": site,
        "servers": servers,
        "networks": networks,
    });
    json_result(&result)
}

pub(crate) async fn handle_get_client(
    brain: &super::OpsBrain,
    p: GetClientParams,
) -> CallToolResult {
    match crate::repo::client_repo::get_client_by_slug(&brain.pool, &p.slug).await {
        Ok(Some(client)) => json_result(&client),
        Ok(None) => not_found_with_suggestions(&brain.pool, "Client", &p.slug).await,
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_get_network(
    brain: &super::OpsBrain,
    p: GetNetworkParams,
) -> CallToolResult {
    if let Some(id_str) = &p.id {
        let id = match uuid::Uuid::parse_str(id_str) {
            Ok(id) => id,
            Err(_) => return error_result(&format!("Invalid UUID: {id_str}")),
        };
        return match crate::repo::network_repo::get_network(&brain.pool, id).await {
            Ok(Some(network)) => json_result(&network),
            Ok(None) => not_found("Network", id_str),
            Err(e) => error_result(&format!("Database error: {e}")),
        };
    }
    let site_id = match &p.site_slug {
        Some(slug) => match crate::repo::site_repo::get_site_by_slug(&brain.pool, slug).await {
            Ok(Some(s)) => Some(s.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Site", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };
    match crate::repo::network_repo::list_networks(&brain.pool, site_id).await {
        Ok(networks) => json_result(&networks),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

/// Resolve a vendor by ID (preferred) or name. Returns Ok(vendor) or a CallToolResult error.
async fn resolve_vendor(
    pool: &sqlx::PgPool,
    id: Option<&str>,
    name: Option<&str>,
) -> Result<crate::models::vendor::Vendor, CallToolResult> {
    if let Some(id_str) = id {
        let uuid = match uuid::Uuid::parse_str(id_str) {
            Ok(u) => u,
            Err(_) => return Err(error_result(&format!("Invalid UUID: {id_str}"))),
        };
        return match crate::repo::vendor_repo::get_vendor(pool, uuid).await {
            Ok(Some(v)) if v.status != "deleted" => Ok(v),
            Ok(_) => Err(not_found("Vendor", id_str)),
            Err(e) => Err(error_result(&format!("Database error: {e}"))),
        };
    }
    if let Some(name_str) = name {
        return match crate::repo::vendor_repo::get_vendor_by_name(pool, name_str).await {
            Ok(Some(v)) => Ok(v),
            Ok(None) => Err(not_found_vendor_with_suggestions(pool, name_str).await),
            Err(e) => Err(error_result(&format!("Database error: {e}"))),
        };
    }
    Err(error_result("Either 'id' or 'name' must be provided"))
}

pub(crate) async fn handle_get_vendor(
    brain: &super::OpsBrain,
    p: GetVendorParams,
) -> CallToolResult {
    match resolve_vendor(&brain.pool, p.id.as_deref(), p.name.as_deref()).await {
        Ok(vendor) => json_result(&vendor),
        Err(err) => err,
    }
}

pub(crate) async fn handle_search_inventory(
    brain: &super::OpsBrain,
    p: SearchInventoryParams,
) -> CallToolResult {
    let limit = p.limit.unwrap_or(10);
    let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

    // Resolve client scope
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    match crate::repo::search_repo::search_inventory(&brain.pool, &p.query, limit).await {
        Ok(results) => {
            let mut output = serde_json::json!({});

            // Ungated entity types — pass through directly
            output["servers"] = serde_json::to_value(&results.servers).unwrap_or_default();
            output["services"] = serde_json::to_value(&results.services).unwrap_or_default();
            output["vendors"] = serde_json::to_value(&results.vendors).unwrap_or_default();
            output["clients"] = serde_json::to_value(&results.clients).unwrap_or_default();
            output["sites"] = serde_json::to_value(&results.sites).unwrap_or_default();
            output["networks"] = serde_json::to_value(&results.networks).unwrap_or_default();
            output["handoffs"] = serde_json::to_value(&results.handoffs).unwrap_or_default();

            // Gated entity types — apply cross-client filter
            use super::helpers::filter_cross_client;
            use super::shared::{build_client_lookup, log_audit_entries};

            let client_lookup = build_client_lookup(&brain.pool).await;

            let runbook_values: Vec<serde_json::Value> = results
                .runbooks
                .iter()
                .filter_map(|r| serde_json::to_value(r).ok())
                .collect();
            let rb_filtered = filter_cross_client(
                runbook_values,
                "runbook",
                client_id,
                acknowledge,
                &client_lookup,
            );
            log_audit_entries(
                &brain.pool,
                "search_inventory",
                client_id,
                "runbook",
                &rb_filtered.audit_entries,
            )
            .await;

            let knowledge_values: Vec<serde_json::Value> = results
                .knowledge
                .iter()
                .filter_map(|k| serde_json::to_value(k).ok())
                .collect();
            let kn_filtered = filter_cross_client(
                knowledge_values,
                "knowledge",
                client_id,
                acknowledge,
                &client_lookup,
            );
            log_audit_entries(
                &brain.pool,
                "search_inventory",
                client_id,
                "knowledge",
                &kn_filtered.audit_entries,
            )
            .await;

            let incident_values: Vec<serde_json::Value> = results
                .incidents
                .iter()
                .filter_map(|i| serde_json::to_value(i).ok())
                .collect();
            let inc_filtered = filter_cross_client(
                incident_values,
                "incident",
                client_id,
                acknowledge,
                &client_lookup,
            );
            log_audit_entries(
                &brain.pool,
                "search_inventory",
                client_id,
                "incident",
                &inc_filtered.audit_entries,
            )
            .await;

            output["runbooks"] = serde_json::to_value(&rb_filtered.allowed).unwrap_or_default();
            output["knowledge"] = serde_json::to_value(&kn_filtered.allowed).unwrap_or_default();
            output["incidents"] = serde_json::to_value(&inc_filtered.allowed).unwrap_or_default();

            // Collect withheld notices
            let mut withheld: Vec<serde_json::Value> = Vec::new();
            withheld.extend(rb_filtered.withheld_notices);
            withheld.extend(kn_filtered.withheld_notices);
            withheld.extend(inc_filtered.withheld_notices);
            if !withheld.is_empty() {
                output["cross_client_withheld"] = serde_json::json!(withheld);
            }

            super::helpers::json_result(&output)
        }
        Err(e) => error_result(&format!("Search error: {e}")),
    }
}

pub(crate) async fn handle_upsert_client(
    brain: &super::OpsBrain,
    p: UpsertClientParams,
) -> CallToolResult {
    match crate::repo::client_repo::upsert_client(
        &brain.pool,
        &p.name,
        &p.slug,
        p.notes.as_deref(),
        p.zammad_org_id,
        p.zammad_group_id,
        p.zammad_customer_id,
    )
    .await
    {
        Ok(client) => json_result(&client),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_upsert_site(
    brain: &super::OpsBrain,
    p: UpsertSiteParams,
) -> CallToolResult {
    let client = match crate::repo::client_repo::get_client_by_slug(&brain.pool, &p.client_slug)
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", &p.client_slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    match crate::repo::site_repo::upsert_site(
        &brain.pool,
        client.id,
        &p.name,
        &p.slug,
        p.address.as_deref(),
        p.wan_provider.as_deref(),
        p.wan_ip.as_deref(),
        p.notes.as_deref(),
    )
    .await
    {
        Ok(site) => json_result(&site),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_upsert_server(
    brain: &super::OpsBrain,
    p: UpsertServerParams,
) -> CallToolResult {
    let site = match crate::repo::site_repo::get_site_by_slug(&brain.pool, &p.site_slug).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Site", &p.site_slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    let hypervisor_id = match &p.hypervisor_slug {
        Some(slug) => match crate::repo::server_repo::get_server_by_slug(&brain.pool, slug).await {
            Ok(Some(h)) => Some(Some(h.id)),
            Ok(None) => {
                return not_found_with_suggestions(&brain.pool, "Hypervisor server", slug).await
            }
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None, // Not provided — don't touch hypervisor_id on update
    };

    // Check if server already exists — route to partial update to avoid nuking fields
    let existing = crate::repo::server_repo::get_server_by_slug(&brain.pool, &p.slug).await;
    match existing {
        Ok(Some(_)) => {
            // Partial update: only provided fields are changed
            match crate::repo::server_repo::update_server_partial(
                &brain.pool,
                &p.slug,
                Some(site.id),
                Some(&p.hostname),
                p.os.as_deref(),
                p.ip_addresses.as_deref(),
                p.ssh_alias.as_deref(),
                p.roles.as_deref(),
                p.hardware.as_deref(),
                p.cpu.as_deref(),
                p.ram_gb,
                p.storage_summary.as_deref(),
                p.is_virtual,
                hypervisor_id,
                p.status.as_deref(),
                p.notes.as_deref(),
            )
            .await
            {
                Ok(server) => json_result(&server),
                Err(e) => error_result(&format!("Database error: {e}")),
            }
        }
        Ok(None) => {
            // New server: apply defaults for NOT NULL fields
            let ip_addresses = p.ip_addresses.unwrap_or_default();
            let roles = p.roles.unwrap_or_default();
            let is_virtual = p.is_virtual.unwrap_or(false);
            let status = p.status.as_deref().unwrap_or("active");
            match crate::repo::server_repo::upsert_server(
                &brain.pool,
                site.id,
                &p.hostname,
                &p.slug,
                p.os.as_deref(),
                &ip_addresses,
                p.ssh_alias.as_deref(),
                &roles,
                p.hardware.as_deref(),
                p.cpu.as_deref(),
                p.ram_gb,
                p.storage_summary.as_deref(),
                is_virtual,
                hypervisor_id.flatten(),
                status,
                p.notes.as_deref(),
            )
            .await
            {
                Ok(server) => json_result(&server),
                Err(e) => error_result(&format!("Database error: {e}")),
            }
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_upsert_service(
    brain: &super::OpsBrain,
    p: UpsertServiceParams,
) -> CallToolResult {
    let criticality = p.criticality.as_deref().unwrap_or("medium");
    match crate::repo::service_repo::upsert_service(
        &brain.pool,
        &p.name,
        &p.slug,
        p.category.as_deref(),
        p.description.as_deref(),
        criticality,
        p.notes.as_deref(),
    )
    .await
    {
        Ok(service) => {
            // Mark as verified if requested
            if p.verified.unwrap_or(false) {
                if let Err(e) =
                    crate::repo::service_repo::update_last_verified_at(&brain.pool, service.id)
                        .await
                {
                    tracing::warn!(
                        "Failed to update last_verified_at for service {}: {e}",
                        service.id
                    );
                }
            }
            json_result(&service)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_upsert_vendor(
    brain: &super::OpsBrain,
    p: UpsertVendorParams,
) -> CallToolResult {
    let contract_end = match &p.contract_end {
        Some(date_str) => match chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
            Ok(d) => Some(d),
            Err(_) => {
                return error_result(&format!(
                    "Invalid date format '{}', expected YYYY-MM-DD",
                    date_str
                ))
            }
        },
        None => None,
    };

    // Resolve client_slug to client_id for auto-linking
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    // If ID is provided, update that specific vendor
    if let Some(id_str) = &p.id {
        let id = match uuid::Uuid::parse_str(id_str) {
            Ok(u) => u,
            Err(_) => return error_result(&format!("Invalid UUID: {id_str}")),
        };
        return match crate::repo::vendor_repo::update_vendor_by_id(
            &brain.pool,
            id,
            &p.name,
            p.category.as_deref(),
            p.account_number.as_deref(),
            p.support_phone.as_deref(),
            p.support_email.as_deref(),
            p.support_portal.as_deref(),
            p.sla_summary.as_deref(),
            contract_end,
            p.notes.as_deref(),
        )
        .await
        {
            Ok(vendor) => {
                // Auto-link vendor to client if client_slug was provided
                if let Some(cid) = client_id {
                    let _ =
                        crate::repo::vendor_repo::link_vendor_client(&brain.pool, vendor.id, cid)
                            .await;
                }
                json_result(&vendor)
            }
            Err(e) => error_result(&format!("Database error: {e}")),
        };
    }
    match crate::repo::vendor_repo::upsert_vendor(
        &brain.pool,
        &p.name,
        p.category.as_deref(),
        p.account_number.as_deref(),
        p.support_phone.as_deref(),
        p.support_email.as_deref(),
        p.support_portal.as_deref(),
        p.sla_summary.as_deref(),
        contract_end,
        p.notes.as_deref(),
    )
    .await
    {
        Ok(vendor) => {
            // Auto-link vendor to client if client_slug was provided
            if let Some(cid) = client_id {
                let _ =
                    crate::repo::vendor_repo::link_vendor_client(&brain.pool, vendor.id, cid).await;
            }
            json_result(&vendor)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_upsert_network(
    brain: &super::OpsBrain,
    p: UpsertNetworkParams,
) -> CallToolResult {
    let site = match crate::repo::site_repo::get_site_by_slug(&brain.pool, &p.site_slug).await {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Site", &p.site_slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    let dns_servers = p.dns_servers.unwrap_or_default();

    match crate::repo::network_repo::upsert_network(
        &brain.pool,
        site.id,
        &p.name,
        &p.cidr,
        p.vlan_id,
        p.gateway.as_deref(),
        &dns_servers,
        p.dhcp_server.as_deref(),
        p.purpose.as_deref(),
        p.notes.as_deref(),
    )
    .await
    {
        Ok(network) => json_result(&network),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_link_server_service(
    brain: &super::OpsBrain,
    p: LinkServerServiceParams,
) -> CallToolResult {
    let server = match crate::repo::server_repo::get_server_by_slug(&brain.pool, &p.server_slug)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Server", &p.server_slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    let service =
        match crate::repo::service_repo::get_service_by_slug(&brain.pool, &p.service_slug).await {
            Ok(Some(s)) => s,
            Ok(None) => {
                return not_found_with_suggestions(&brain.pool, "Service", &p.service_slug).await
            }
            Err(e) => return error_result(&format!("Database error: {e}")),
        };
    match crate::repo::service_repo::link_server_service(
        &brain.pool,
        server.id,
        service.id,
        p.port,
        p.config_notes.as_deref(),
    )
    .await
    {
        Ok(()) => CallToolResult::success(vec![Content::text(format!(
            "Linked server '{}' to service '{}'",
            p.server_slug, p.service_slug
        ))]),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_list_vendors(
    brain: &super::OpsBrain,
    p: ListVendorsParams,
) -> CallToolResult {
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };
    match crate::repo::vendor_repo::list_vendors(&brain.pool, client_id, p.category.as_deref())
        .await
    {
        Ok(mut vendors) => {
            let limit = p.limit.unwrap_or(50) as usize;
            vendors.truncate(limit);
            json_result(&vendors)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_list_clients(
    brain: &super::OpsBrain,
    p: ListClientsParams,
) -> CallToolResult {
    match crate::repo::client_repo::list_clients(&brain.pool).await {
        Ok(mut clients) => {
            let limit = p.limit.unwrap_or(50) as usize;
            clients.truncate(limit);
            json_result(&clients)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_list_sites(
    brain: &super::OpsBrain,
    p: ListSitesParams,
) -> CallToolResult {
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };
    match crate::repo::site_repo::list_sites(&brain.pool, client_id).await {
        Ok(mut sites) => {
            let limit = p.limit.unwrap_or(50) as usize;
            sites.truncate(limit);
            json_result(&sites)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_list_networks(
    brain: &super::OpsBrain,
    p: ListNetworksParams,
) -> CallToolResult {
    let site_id = match &p.site_slug {
        Some(slug) => match crate::repo::site_repo::get_site_by_slug(&brain.pool, slug).await {
            Ok(Some(s)) => Some(s.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Site", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };
    match crate::repo::network_repo::list_networks(&brain.pool, site_id).await {
        Ok(mut networks) => {
            let limit = p.limit.unwrap_or(50) as usize;
            networks.truncate(limit);
            json_result(&networks)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_delete_server(
    brain: &super::OpsBrain,
    params: DeleteServerParams,
) -> CallToolResult {
    let server = match crate::repo::server_repo::get_server_by_slug(&brain.pool, &params.slug).await
    {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Server", &params.slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    let refs = match crate::repo::server_repo::count_server_references(&brain.pool, server.id).await
    {
        Ok(r) => r,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    if params.confirm != Some(true) {
        let mut preview = serde_json::json!({
            "action": "delete_server",
            "server": server.hostname,
            "slug": server.slug,
            "status": server.status,
            "confirmed": false,
            "message": "Pass confirm=true to proceed with deletion.",
        });
        if !refs.is_empty() {
            let ref_map: serde_json::Map<String, serde_json::Value> = refs
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
                .collect();
            preview["linked_entities"] = serde_json::Value::Object(ref_map);
            preview["warning"] =
                "Entity will be soft-deleted (status='deleted'). FK references preserved.".into();
        } else {
            preview["linked_entities"] = serde_json::json!("none");
        }
        return json_result(&preview);
    }
    match crate::repo::server_repo::delete_server(&brain.pool, server.id).await {
        Ok(true) => json_result(&serde_json::json!({
            "deleted": true,
            "soft_delete": true,
            "server": server.hostname,
            "slug": server.slug,
            "note": "Server marked as deleted (soft delete). FK references preserved.",
        })),
        Ok(false) => not_found("Server", &params.slug),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_delete_service(
    brain: &super::OpsBrain,
    params: DeleteServiceParams,
) -> CallToolResult {
    let service = match crate::repo::service_repo::get_service_by_slug(&brain.pool, &params.slug)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Service", &params.slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    let refs =
        match crate::repo::service_repo::count_service_references(&brain.pool, service.id).await {
            Ok(r) => r,
            Err(e) => return error_result(&format!("Database error: {e}")),
        };
    if params.confirm != Some(true) {
        let mut preview = serde_json::json!({
            "action": "delete_service",
            "service": service.name,
            "slug": service.slug,
            "confirmed": false,
            "message": "Pass confirm=true to proceed with deletion.",
        });
        if !refs.is_empty() {
            let ref_map: serde_json::Map<String, serde_json::Value> = refs
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
                .collect();
            preview["linked_entities"] = serde_json::Value::Object(ref_map);
            preview["warning"] =
                "Entity will be soft-deleted (status='deleted'). FK references preserved.".into();
        } else {
            preview["linked_entities"] = serde_json::json!("none");
        }
        return json_result(&preview);
    }
    match crate::repo::service_repo::delete_service(&brain.pool, service.id).await {
        Ok(true) => json_result(&serde_json::json!({
            "deleted": true,
            "soft_delete": true,
            "service": service.name,
            "slug": service.slug,
            "note": "Service marked as deleted (soft delete). FK references preserved.",
        })),
        Ok(false) => not_found("Service", &params.slug),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_delete_vendor(
    brain: &super::OpsBrain,
    params: DeleteVendorParams,
) -> CallToolResult {
    let vendor =
        match resolve_vendor(&brain.pool, params.id.as_deref(), params.name.as_deref()).await {
            Ok(v) => v,
            Err(err) => return err,
        };
    let refs = match crate::repo::vendor_repo::count_vendor_references(&brain.pool, vendor.id).await
    {
        Ok(r) => r,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    if params.confirm != Some(true) {
        let mut preview = serde_json::json!({
            "action": "delete_vendor",
            "vendor": vendor.name,
            "id": vendor.id.to_string(),
            "confirmed": false,
            "message": "Pass confirm=true to proceed with deletion.",
        });
        if !refs.is_empty() {
            let ref_map: serde_json::Map<String, serde_json::Value> = refs
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::Value::from(*v)))
                .collect();
            preview["linked_entities"] = serde_json::Value::Object(ref_map);
            preview["warning"] =
                "Entity will be soft-deleted (status='deleted'). FK references preserved.".into();
        } else {
            preview["linked_entities"] = serde_json::json!("none");
        }
        return json_result(&preview);
    }
    match crate::repo::vendor_repo::delete_vendor(&brain.pool, vendor.id).await {
        Ok(true) => json_result(&serde_json::json!({
            "deleted": true,
            "soft_delete": true,
            "vendor": vendor.name,
            "id": vendor.id.to_string(),
            "note": "Vendor marked as deleted (soft delete). FK references preserved.",
        })),
        Ok(false) => not_found("Vendor", &vendor.id.to_string()),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}
