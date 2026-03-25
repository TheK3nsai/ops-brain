pub mod briefings;
mod context;
mod coordination;
mod incidents;
mod inventory;
mod knowledge;
mod monitoring;
mod runbooks;
mod search;
mod zammad;

use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::Serialize;
use sqlx::PgPool;

use std::collections::HashMap;

use crate::embeddings::EmbeddingClient;
use crate::metrics::UptimeKumaConfig;
use crate::models::handoff::Handoff;
use crate::models::incident::Incident;
use crate::zammad::ZammadConfig;

#[derive(Clone)]
pub struct OpsBrain {
    pool: PgPool,
    kuma_config: Option<UptimeKumaConfig>,
    embedding_client: Option<EmbeddingClient>,
    zammad_config: Option<ZammadConfig>,
    tool_router: ToolRouter<Self>,
}

// Helper to format tool results as JSON text content
fn json_result<T: Serialize>(data: &T) -> CallToolResult {
    match serde_json::to_string_pretty(data) {
        Ok(json) => CallToolResult::success(vec![Content::text(json)]),
        Err(e) => CallToolResult::error(vec![Content::text(format!("Serialization error: {e}"))]),
    }
}

fn error_result(msg: &str) -> CallToolResult {
    CallToolResult::error(vec![Content::text(msg.to_string())])
}

fn not_found(entity: &str, key: &str) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("{entity} not found: {key}"))])
}

/// Result of cross-client scope filtering.
struct CrossClientFilterResult {
    /// Items that passed the gate (with _provenance fields injected)
    pub allowed: Vec<serde_json::Value>,
    /// Grouped notices about withheld content (for response)
    pub withheld_notices: Vec<serde_json::Value>,
    /// Individual (entity_id, owning_client_id, action) for audit logging
    pub audit_entries: Vec<(uuid::Uuid, Option<uuid::Uuid>, String)>,
}

/// Partition items into allowed and withheld based on cross-client scope.
///
/// Rules:
/// - No requesting client → all items allowed (no scope to enforce)
/// - Item client_id is NULL → allowed (global content)
/// - Item client_id == requesting → allowed (same client)
/// - Item client_id != requesting + cross_client_safe=true → allowed
/// - Item client_id != requesting + cross_client_safe=false + acknowledge=true → allowed (released)
/// - Item client_id != requesting + cross_client_safe=false + acknowledge=false → WITHHELD
fn filter_cross_client(
    items: Vec<serde_json::Value>,
    entity_type: &str,
    requesting_client_id: Option<uuid::Uuid>,
    acknowledge: bool,
    client_lookup: &HashMap<uuid::Uuid, (String, String)>, // client_id -> (slug, name)
) -> CrossClientFilterResult {
    let Some(req_cid) = requesting_client_id else {
        // No requesting client scope — all items are allowed, inject provenance
        let allowed = items
            .into_iter()
            .map(|mut item| {
                inject_provenance(&mut item, client_lookup);
                item
            })
            .collect();
        return CrossClientFilterResult {
            allowed,
            withheld_notices: Vec::new(),
            audit_entries: Vec::new(),
        };
    };

    let mut allowed = Vec::new();
    let mut withheld_by_client: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();
    let mut audit_entries = Vec::new();

    for mut item in items {
        let item_client_id = item
            .get("client_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        let cross_client_safe = item
            .get("cross_client_safe")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let entity_id = item
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        match item_client_id {
            // Global content (no client_id) — always allowed
            None => {
                inject_provenance(&mut item, client_lookup);
                allowed.push(item);
            }
            // Same client — allowed
            Some(cid) if cid == req_cid => {
                inject_provenance(&mut item, client_lookup);
                allowed.push(item);
            }
            // Different client but marked safe — allowed
            Some(cid) if cross_client_safe => {
                inject_provenance(&mut item, client_lookup);
                allowed.push(item);
                if let Some(eid) = entity_id {
                    audit_entries.push((eid, Some(cid), "released_safe".to_string()));
                }
            }
            // Different client, not safe, but acknowledged — released
            Some(cid) if acknowledge => {
                inject_provenance(&mut item, client_lookup);
                allowed.push(item);
                if let Some(eid) = entity_id {
                    audit_entries.push((eid, Some(cid), "released".to_string()));
                }
            }
            // Different client, not safe, not acknowledged — WITHHELD
            Some(cid) => {
                if let Some(eid) = entity_id {
                    withheld_by_client.entry(cid).or_default().push(eid);
                    audit_entries.push((eid, Some(cid), "withheld".to_string()));
                }
            }
        }
    }

    // Build grouped withheld notices
    let withheld_notices: Vec<serde_json::Value> = withheld_by_client
        .into_iter()
        .map(|(cid, entity_ids)| {
            let (slug, name) = client_lookup
                .get(&cid)
                .cloned()
                .unwrap_or_else(|| ("unknown".to_string(), "Unknown".to_string()));
            serde_json::json!({
                "entity_type": entity_type,
                "count": entity_ids.len(),
                "owning_client_slug": slug,
                "owning_client_name": name,
                "message": format!(
                    "{} {}(s) from client '{}' withheld — cross-client scope mismatch. Re-call with acknowledge_cross_client: true to release.",
                    entity_ids.len(), entity_type, name
                )
            })
        })
        .collect();

    CrossClientFilterResult {
        allowed,
        withheld_notices,
        audit_entries,
    }
}

/// Inject _provenance fields (client slug + name) into a JSON item.
fn inject_provenance(
    item: &mut serde_json::Value,
    client_lookup: &HashMap<uuid::Uuid, (String, String)>,
) {
    if let Some(obj) = item.as_object_mut() {
        let client_id = obj
            .get("client_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok());
        match client_id {
            Some(cid) => {
                if let Some((slug, name)) = client_lookup.get(&cid) {
                    obj.insert(
                        "_client_slug".to_string(),
                        serde_json::Value::String(slug.clone()),
                    );
                    obj.insert(
                        "_client_name".to_string(),
                        serde_json::Value::String(name.clone()),
                    );
                }
            }
            None => {
                obj.insert("_client_slug".to_string(), serde_json::Value::Null);
                obj.insert(
                    "_client_name".to_string(),
                    serde_json::Value::String("Global".to_string()),
                );
            }
        }
    }
}

#[tool_router]
impl OpsBrain {
    pub fn new(
        pool: PgPool,
        kuma_config: Option<UptimeKumaConfig>,
        embedding_client: Option<EmbeddingClient>,
        zammad_config: Option<ZammadConfig>,
    ) -> Self {
        Self {
            pool,
            kuma_config,
            embedding_client,
            zammad_config,
            tool_router: Self::tool_router(),
        }
    }

    /// Best-effort embed and store: logs warning on failure, never blocks the caller.
    async fn embed_and_store(&self, table: &str, id: uuid::Uuid, text: &str) {
        let Some(ref client) = self.embedding_client else {
            return;
        };
        match client.embed_text(text).await {
            Ok(embedding) => {
                let result = match table {
                    "runbooks" => {
                        crate::repo::embedding_repo::store_runbook_embedding(
                            &self.pool, id, &embedding,
                        )
                        .await
                    }
                    "knowledge" => {
                        crate::repo::embedding_repo::store_knowledge_embedding(
                            &self.pool, id, &embedding,
                        )
                        .await
                    }
                    "incidents" => {
                        crate::repo::embedding_repo::store_incident_embedding(
                            &self.pool, id, &embedding,
                        )
                        .await
                    }
                    "handoffs" => {
                        crate::repo::embedding_repo::store_handoff_embedding(
                            &self.pool, id, &embedding,
                        )
                        .await
                    }
                    _ => return,
                };
                if let Err(e) = result {
                    tracing::warn!("Failed to store embedding for {table}/{id}: {e}");
                }
            }
            Err(e) => {
                tracing::warn!("Failed to generate embedding for {table}/{id}: {e}");
            }
        }
    }

    /// Helper to get query embedding, returning None if embedding client unavailable.
    async fn get_query_embedding(&self, text: &str) -> Option<Vec<f32>> {
        let client = self.embedding_client.as_ref()?;
        match client.embed_text(text).await {
            Ok(emb) => Some(emb),
            Err(e) => {
                tracing::warn!("Failed to embed query: {e}");
                None
            }
        }
    }

    /// Build a client_id -> (slug, name) lookup from the database.
    async fn build_client_lookup(&self) -> HashMap<uuid::Uuid, (String, String)> {
        match crate::repo::client_repo::list_clients(&self.pool).await {
            Ok(clients) => clients
                .into_iter()
                .map(|c| (c.id, (c.slug, c.name)))
                .collect(),
            Err(e) => {
                tracing::warn!("Failed to build client lookup: {e}");
                HashMap::new()
            }
        }
    }

    /// Write audit log entries for cross-client filtering results.
    async fn log_audit_entries(
        &self,
        tool_name: &str,
        requesting_client_id: Option<uuid::Uuid>,
        entity_type: &str,
        entries: &[(uuid::Uuid, Option<uuid::Uuid>, String)],
    ) {
        for (entity_id, owning_client_id, action) in entries {
            crate::repo::audit_log_repo::log_access(
                &self.pool,
                tool_name,
                requesting_client_id,
                entity_type,
                *entity_id,
                *owning_client_id,
                action,
            )
            .await;
        }
    }

    // ===== INVENTORY: READ TOOLS =====

    #[tool(
        name = "get_server",
        description = "Get detailed information about a server including its services, site, and network configuration"
    )]
    async fn get_server(
        &self,
        params: Parameters<inventory::GetServerParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let server = match crate::repo::server_repo::get_server_by_slug(&self.pool, &p.slug).await {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(not_found("Server", &p.slug)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        };
        let services = crate::repo::service_repo::get_services_for_server(&self.pool, server.id)
            .await
            .unwrap_or_default();
        let site = crate::repo::site_repo::get_site(&self.pool, server.site_id)
            .await
            .ok()
            .flatten();
        let networks = crate::repo::network_repo::list_networks(&self.pool, Some(server.site_id))
            .await
            .unwrap_or_default();

        let result = serde_json::json!({
            "server": server,
            "services": services,
            "site": site,
            "networks": networks,
        });
        Ok(json_result(&result))
    }

    #[tool(
        name = "list_servers",
        description = "List servers with optional filters by client, site, role, or status"
    )]
    async fn list_servers(
        &self,
        params: Parameters<inventory::ListServersParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // Resolve client_slug to client_id
        let client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        // Resolve site_slug to site_id
        let site_id = match &p.site_slug {
            Some(slug) => match crate::repo::site_repo::get_site_by_slug(&self.pool, slug).await {
                Ok(Some(s)) => Some(s.id),
                Ok(None) => return Ok(not_found("Site", slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            },
            None => None,
        };

        match crate::repo::server_repo::list_servers(
            &self.pool,
            client_id,
            site_id,
            p.role.as_deref(),
            p.status.as_deref(),
        )
        .await
        {
            Ok(servers) => Ok(json_result(&servers)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "get_service",
        description = "Get detailed information about a service and which servers run it"
    )]
    async fn get_service(
        &self,
        params: Parameters<inventory::GetServiceParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let service =
            match crate::repo::service_repo::get_service_by_slug(&self.pool, &p.slug).await {
                Ok(Some(s)) => s,
                Ok(None) => return Ok(not_found("Service", &p.slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };
        let servers = crate::repo::service_repo::get_servers_for_service(&self.pool, service.id)
            .await
            .unwrap_or_default();

        let result = serde_json::json!({
            "service": service,
            "servers": servers,
        });
        Ok(json_result(&result))
    }

    #[tool(
        name = "list_services",
        description = "List all services, optionally filtered by category"
    )]
    async fn list_services(
        &self,
        params: Parameters<inventory::ListServicesParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::service_repo::list_services(&self.pool, p.category.as_deref()).await {
            Ok(services) => Ok(json_result(&services)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "get_site",
        description = "Get detailed information about a site including its servers and networks"
    )]
    async fn get_site(
        &self,
        params: Parameters<inventory::GetSiteParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let site = match crate::repo::site_repo::get_site_by_slug(&self.pool, &p.slug).await {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(not_found("Site", &p.slug)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        };
        let servers =
            crate::repo::server_repo::list_servers(&self.pool, None, Some(site.id), None, None)
                .await
                .unwrap_or_default();
        let networks = crate::repo::network_repo::list_networks(&self.pool, Some(site.id))
            .await
            .unwrap_or_default();

        let result = serde_json::json!({
            "site": site,
            "servers": servers,
            "networks": networks,
        });
        Ok(json_result(&result))
    }

    #[tool(name = "get_client", description = "Get client information by slug")]
    async fn get_client(
        &self,
        params: Parameters<inventory::GetClientParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::client_repo::get_client_by_slug(&self.pool, &p.slug).await {
            Ok(Some(client)) => Ok(json_result(&client)),
            Ok(None) => Ok(not_found("Client", &p.slug)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "get_network",
        description = "Get network information by site slug or network ID"
    )]
    async fn get_network(
        &self,
        params: Parameters<inventory::GetNetworkParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // If ID is provided, look up directly
        if let Some(id_str) = &p.id {
            let id = match uuid::Uuid::parse_str(id_str) {
                Ok(id) => id,
                Err(_) => return Ok(error_result(&format!("Invalid UUID: {id_str}"))),
            };
            return match crate::repo::network_repo::get_network(&self.pool, id).await {
                Ok(Some(network)) => Ok(json_result(&network)),
                Ok(None) => Ok(not_found("Network", id_str)),
                Err(e) => Ok(error_result(&format!("Database error: {e}"))),
            };
        }

        // Otherwise filter by site_slug
        let site_id = match &p.site_slug {
            Some(slug) => match crate::repo::site_repo::get_site_by_slug(&self.pool, slug).await {
                Ok(Some(s)) => Some(s.id),
                Ok(None) => return Ok(not_found("Site", slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            },
            None => None,
        };

        match crate::repo::network_repo::list_networks(&self.pool, site_id).await {
            Ok(networks) => Ok(json_result(&networks)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "get_vendor",
        description = "Get vendor information by name (case-insensitive)"
    )]
    async fn get_vendor(
        &self,
        params: Parameters<inventory::GetVendorParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::vendor_repo::get_vendor_by_name(&self.pool, &p.name).await {
            Ok(Some(vendor)) => Ok(json_result(&vendor)),
            Ok(None) => Ok(not_found("Vendor", &p.name)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "search_inventory",
        description = "Full-text search across servers, services, runbooks, and knowledge entries"
    )]
    async fn search_inventory(
        &self,
        params: Parameters<inventory::SearchInventoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::search_repo::search_inventory(&self.pool, &p.query).await {
            Ok(results) => Ok(json_result(&results)),
            Err(e) => Ok(error_result(&format!("Search error: {e}"))),
        }
    }

    // ===== INVENTORY: WRITE TOOLS =====

    #[tool(
        name = "upsert_client",
        description = "Create or update a client (organization). Updates existing if slug matches."
    )]
    async fn upsert_client(
        &self,
        params: Parameters<inventory::UpsertClientParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::client_repo::upsert_client(
            &self.pool,
            &p.name,
            &p.slug,
            p.notes.as_deref(),
            p.zammad_org_id,
            p.zammad_group_id,
            p.zammad_customer_id,
        )
        .await
        {
            Ok(client) => Ok(json_result(&client)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "upsert_site",
        description = "Create or update a site. Resolves client_slug to find the parent client."
    )]
    async fn upsert_site(
        &self,
        params: Parameters<inventory::UpsertSiteParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let client =
            match crate::repo::client_repo::get_client_by_slug(&self.pool, &p.client_slug).await {
                Ok(Some(c)) => c,
                Ok(None) => return Ok(not_found("Client", &p.client_slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        match crate::repo::site_repo::upsert_site(
            &self.pool,
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
            Ok(site) => Ok(json_result(&site)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "upsert_server",
        description = "Create or update a server. Resolves site_slug and optional hypervisor_slug to IDs."
    )]
    async fn upsert_server(
        &self,
        params: Parameters<inventory::UpsertServerParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let site = match crate::repo::site_repo::get_site_by_slug(&self.pool, &p.site_slug).await {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(not_found("Site", &p.site_slug)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        };

        // Resolve optional hypervisor_slug
        let hypervisor_id = match &p.hypervisor_slug {
            Some(slug) => {
                match crate::repo::server_repo::get_server_by_slug(&self.pool, slug).await {
                    Ok(Some(h)) => Some(h.id),
                    Ok(None) => return Ok(not_found("Hypervisor server", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        let ip_addresses = p.ip_addresses.unwrap_or_default();
        let roles = p.roles.unwrap_or_default();
        let is_virtual = p.is_virtual.unwrap_or(false);
        let status = p.status.as_deref().unwrap_or("active");

        match crate::repo::server_repo::upsert_server(
            &self.pool,
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
            hypervisor_id,
            status,
            p.notes.as_deref(),
        )
        .await
        {
            Ok(server) => Ok(json_result(&server)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "upsert_service",
        description = "Create or update a service definition. Updates existing if slug matches."
    )]
    async fn upsert_service(
        &self,
        params: Parameters<inventory::UpsertServiceParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let criticality = p.criticality.as_deref().unwrap_or("medium");

        match crate::repo::service_repo::upsert_service(
            &self.pool,
            &p.name,
            &p.slug,
            p.category.as_deref(),
            p.description.as_deref(),
            criticality,
            p.notes.as_deref(),
        )
        .await
        {
            Ok(service) => Ok(json_result(&service)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "upsert_vendor",
        description = "Create a new vendor contact record"
    )]
    async fn upsert_vendor(
        &self,
        params: Parameters<inventory::UpsertVendorParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // Parse optional contract_end date
        let contract_end = match &p.contract_end {
            Some(date_str) => match chrono::NaiveDate::parse_from_str(date_str, "%Y-%m-%d") {
                Ok(d) => Some(d),
                Err(_) => {
                    return Ok(error_result(&format!(
                        "Invalid date format '{}', expected YYYY-MM-DD",
                        date_str
                    )))
                }
            },
            None => None,
        };

        match crate::repo::vendor_repo::upsert_vendor(
            &self.pool,
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
            Ok(vendor) => Ok(json_result(&vendor)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "link_server_service",
        description = "Link a server to a service it runs, with optional port and config notes"
    )]
    async fn link_server_service(
        &self,
        params: Parameters<inventory::LinkServerServiceParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let server =
            match crate::repo::server_repo::get_server_by_slug(&self.pool, &p.server_slug).await {
                Ok(Some(s)) => s,
                Ok(None) => return Ok(not_found("Server", &p.server_slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };
        let service =
            match crate::repo::service_repo::get_service_by_slug(&self.pool, &p.service_slug).await
            {
                Ok(Some(s)) => s,
                Ok(None) => return Ok(not_found("Service", &p.service_slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        match crate::repo::service_repo::link_server_service(
            &self.pool,
            server.id,
            service.id,
            p.port,
            p.config_notes.as_deref(),
        )
        .await
        {
            Ok(()) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Linked server '{}' to service '{}'",
                p.server_slug, p.service_slug
            ))])),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    // ===== RUNBOOK TOOLS =====

    #[tool(
        name = "get_runbook",
        description = "Get a runbook by its slug, including full content and metadata"
    )]
    async fn get_runbook(
        &self,
        params: Parameters<runbooks::GetRunbookParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::runbook_repo::get_runbook_by_slug(&self.pool, &p.slug).await {
            Ok(Some(runbook)) => Ok(json_result(&runbook)),
            Ok(None) => Ok(not_found("Runbook", &p.slug)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "list_runbooks",
        description = "List runbooks with optional filters by category, service, server, or tag"
    )]
    async fn list_runbooks(
        &self,
        params: Parameters<runbooks::ListRunbooksParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // Resolve optional client_slug
        let client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        // Resolve optional service_slug
        let service_id = match &p.service_slug {
            Some(slug) => {
                match crate::repo::service_repo::get_service_by_slug(&self.pool, slug).await {
                    Ok(Some(s)) => Some(s.id),
                    Ok(None) => return Ok(not_found("Service", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        // Resolve optional server_slug
        let server_id = match &p.server_slug {
            Some(slug) => {
                match crate::repo::server_repo::get_server_by_slug(&self.pool, slug).await {
                    Ok(Some(s)) => Some(s.id),
                    Ok(None) => return Ok(not_found("Server", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        match crate::repo::runbook_repo::list_runbooks(
            &self.pool,
            p.category.as_deref(),
            service_id,
            server_id,
            p.tag.as_deref(),
            client_id,
        )
        .await
        {
            Ok(runbooks) => Ok(json_result(&runbooks)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "search_runbooks",
        description = "Search across runbook titles and content. Supports mode: 'fts' (default, keyword match), \
        'semantic' (AI vector similarity), or 'hybrid' (combined FTS + vector via RRF ranking)"
    )]
    async fn search_runbooks(
        &self,
        params: Parameters<runbooks::SearchRunbooksParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let mode = p.mode.as_deref().unwrap_or("fts");
        if let Err(msg) =
            crate::validation::validate_required(mode, "mode", crate::validation::SEARCH_MODES)
        {
            return Ok(error_result(&msg));
        }

        // Resolve optional client_slug for cross-client gate
        let requesting_client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };
        let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

        let result = match mode {
            "semantic" => {
                let Some(emb) = self.get_query_embedding(&p.query).await else {
                    return Ok(error_result(
                        "Semantic search unavailable (OPENAI_API_KEY not set)",
                    ));
                };
                crate::repo::embedding_repo::vector_search_runbooks(&self.pool, &emb, 20).await
            }
            "hybrid" => {
                let emb = self.get_query_embedding(&p.query).await;
                crate::repo::embedding_repo::hybrid_search_runbooks(
                    &self.pool,
                    &p.query,
                    emb.as_deref(),
                    20,
                )
                .await
            }
            _ => crate::repo::search_repo::search_runbooks(&self.pool, &p.query).await,
        };
        match result {
            Ok(runbooks) => {
                let items: Vec<serde_json::Value> = runbooks
                    .iter()
                    .filter_map(|r| serde_json::to_value(r).ok())
                    .collect();
                let client_lookup = self.build_client_lookup().await;
                let filtered = filter_cross_client(
                    items,
                    "runbook",
                    requesting_client_id,
                    acknowledge,
                    &client_lookup,
                );

                // Log audit entries
                self.log_audit_entries(
                    "search_runbooks",
                    requesting_client_id,
                    "runbook",
                    &filtered.audit_entries,
                )
                .await;

                let mut response = serde_json::json!({ "runbooks": filtered.allowed });
                if !filtered.withheld_notices.is_empty() {
                    response["cross_client_withheld"] =
                        serde_json::json!(filtered.withheld_notices);
                }
                Ok(json_result(&response))
            }
            Err(e) => Ok(error_result(&format!("Search error: {e}"))),
        }
    }

    #[tool(
        name = "create_runbook",
        description = "Create a new runbook with title, slug, content, tags, and metadata"
    )]
    async fn create_runbook(
        &self,
        params: Parameters<runbooks::CreateRunbookParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let tags = p.tags.unwrap_or_default();
        let requires_reboot = p.requires_reboot.unwrap_or(false);
        let cross_client_safe = p.cross_client_safe.unwrap_or(false);

        // Resolve optional client_slug
        let client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        match crate::repo::runbook_repo::create_runbook(
            &self.pool,
            &p.title,
            &p.slug,
            p.category.as_deref(),
            &p.content,
            &tags,
            p.estimated_minutes,
            requires_reboot,
            p.notes.as_deref(),
            client_id,
            cross_client_safe,
        )
        .await
        {
            Ok(runbook) => {
                let text = crate::embeddings::prepare_runbook_text(&runbook);
                self.embed_and_store("runbooks", runbook.id, &text).await;
                Ok(json_result(&runbook))
            }
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "update_runbook",
        description = "Update an existing runbook by slug. Only provided fields are updated; version is auto-incremented."
    )]
    async fn update_runbook(
        &self,
        params: Parameters<runbooks::UpdateRunbookParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let runbook =
            match crate::repo::runbook_repo::get_runbook_by_slug(&self.pool, &p.slug).await {
                Ok(Some(r)) => r,
                Ok(None) => return Ok(not_found("Runbook", &p.slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        // Wrap estimated_minutes in Option<Option<i32>> for COALESCE
        let estimated_minutes: Option<Option<i32>> = p.estimated_minutes.map(Some);

        match crate::repo::runbook_repo::update_runbook(
            &self.pool,
            runbook.id,
            p.title.as_deref(),
            p.category.as_deref(),
            p.content.as_deref(),
            p.tags.as_deref(),
            estimated_minutes,
            p.requires_reboot,
            p.notes.as_deref(),
            p.cross_client_safe,
        )
        .await
        {
            Ok(updated) => {
                let text = crate::embeddings::prepare_runbook_text(&updated);
                self.embed_and_store("runbooks", updated.id, &text).await;
                Ok(json_result(&updated))
            }
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    // ===== KNOWLEDGE TOOLS =====

    #[tool(
        name = "add_knowledge",
        description = "Add a knowledge base entry (lesson learned, gotcha, tip, etc.)"
    )]
    async fn add_knowledge(
        &self,
        params: Parameters<knowledge::AddKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let tags = p.tags.unwrap_or_default();
        let cross_client_safe = p.cross_client_safe.unwrap_or(false);

        // Resolve optional client_slug
        let client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        match crate::repo::knowledge_repo::add_knowledge(
            &self.pool,
            &p.title,
            &p.content,
            p.category.as_deref(),
            &tags,
            client_id,
            cross_client_safe,
        )
        .await
        {
            Ok(entry) => {
                let text = crate::embeddings::prepare_knowledge_text(&entry);
                self.embed_and_store("knowledge", entry.id, &text).await;
                Ok(json_result(&entry))
            }
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "update_knowledge",
        description = "Update an existing knowledge base entry by ID. Only provided fields are updated."
    )]
    async fn update_knowledge(
        &self,
        params: Parameters<knowledge::UpdateKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let id = match uuid::Uuid::parse_str(&p.id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.id))),
        };

        // Verify entry exists
        match crate::repo::knowledge_repo::get_knowledge(&self.pool, id).await {
            Ok(Some(_)) => {}
            Ok(None) => return Ok(not_found("Knowledge", &p.id)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        };

        match crate::repo::knowledge_repo::update_knowledge(
            &self.pool,
            id,
            p.title.as_deref(),
            p.content.as_deref(),
            p.category.as_deref(),
            p.tags.as_deref(),
            p.cross_client_safe,
        )
        .await
        {
            Ok(updated) => {
                let text = crate::embeddings::prepare_knowledge_text(&updated);
                self.embed_and_store("knowledge", updated.id, &text).await;
                Ok(json_result(&updated))
            }
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "delete_knowledge",
        description = "Delete a knowledge base entry by ID. Use with caution — this is permanent."
    )]
    async fn delete_knowledge(
        &self,
        params: Parameters<knowledge::DeleteKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let id = match uuid::Uuid::parse_str(&p.id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.id))),
        };

        match crate::repo::knowledge_repo::delete_knowledge(&self.pool, id).await {
            Ok(true) => Ok(json_result(
                &serde_json::json!({"deleted": true, "id": p.id}),
            )),
            Ok(false) => Ok(not_found("Knowledge", &p.id)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "search_knowledge",
        description = "Search across knowledge base entries. Supports mode: 'fts' (default, keyword match), \
        'semantic' (AI vector similarity), or 'hybrid' (combined FTS + vector via RRF ranking)"
    )]
    async fn search_knowledge(
        &self,
        params: Parameters<knowledge::SearchKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let mode = p.mode.as_deref().unwrap_or("fts");
        if let Err(msg) =
            crate::validation::validate_required(mode, "mode", crate::validation::SEARCH_MODES)
        {
            return Ok(error_result(&msg));
        }

        // Resolve optional client_slug for cross-client gate
        let requesting_client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };
        let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

        let result = match mode {
            "semantic" => {
                let Some(emb) = self.get_query_embedding(&p.query).await else {
                    return Ok(error_result(
                        "Semantic search unavailable (OPENAI_API_KEY not set)",
                    ));
                };
                crate::repo::embedding_repo::vector_search_knowledge(&self.pool, &emb, 20).await
            }
            "hybrid" => {
                let emb = self.get_query_embedding(&p.query).await;
                crate::repo::embedding_repo::hybrid_search_knowledge(
                    &self.pool,
                    &p.query,
                    emb.as_deref(),
                    20,
                )
                .await
            }
            _ => crate::repo::knowledge_repo::search_knowledge(&self.pool, &p.query).await,
        };
        match result {
            Ok(entries) => {
                let items: Vec<serde_json::Value> = entries
                    .iter()
                    .filter_map(|k| serde_json::to_value(k).ok())
                    .collect();
                let client_lookup = self.build_client_lookup().await;
                let filtered = filter_cross_client(
                    items,
                    "knowledge",
                    requesting_client_id,
                    acknowledge,
                    &client_lookup,
                );

                self.log_audit_entries(
                    "search_knowledge",
                    requesting_client_id,
                    "knowledge",
                    &filtered.audit_entries,
                )
                .await;

                let mut response = serde_json::json!({ "knowledge": filtered.allowed });
                if !filtered.withheld_notices.is_empty() {
                    response["cross_client_withheld"] =
                        serde_json::json!(filtered.withheld_notices);
                }
                Ok(json_result(&response))
            }
            Err(e) => Ok(error_result(&format!("Search error: {e}"))),
        }
    }

    #[tool(
        name = "list_knowledge",
        description = "List knowledge base entries, optionally filtered by category or client"
    )]
    async fn list_knowledge(
        &self,
        params: Parameters<knowledge::ListKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // Resolve optional client_slug
        let client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        match crate::repo::knowledge_repo::list_knowledge(
            &self.pool,
            p.category.as_deref(),
            client_id,
        )
        .await
        {
            Ok(entries) => Ok(json_result(&entries)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    // ===== CONTEXT TOOLS =====

    #[tool(
        name = "get_situational_awareness",
        description = "THE KEY TOOL: Get comprehensive situational awareness for a server, service, or client. \
        Gathers all related data: entity details, related entities, recent incidents, pending handoffs, \
        relevant runbooks, vendor contacts, and knowledge entries. Provide at least one of server_slug, \
        service_slug, or client_slug."
    )]
    async fn get_situational_awareness(
        &self,
        params: Parameters<context::GetSituationalAwarenessParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        if p.server_slug.is_none() && p.service_slug.is_none() && p.client_slug.is_none() {
            return Ok(error_result(
                "Provide at least one of: server_slug, service_slug, or client_slug",
            ));
        }

        let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

        let mut awareness = context::SituationalAwareness {
            server: None,
            site: None,
            client: None,
            services: Vec::new(),
            networks: Vec::new(),
            vendors: Vec::new(),
            recent_incidents: Vec::new(),
            relevant_runbooks: Vec::new(),
            pending_handoffs: Vec::new(),
            knowledge: Vec::new(),
            monitoring: Vec::new(),
            linked_tickets: Vec::new(),
            cross_client_withheld: Vec::new(),
        };

        let mut client_id: Option<uuid::Uuid> = None;
        #[allow(unused_assignments)]
        let mut site_id: Option<uuid::Uuid> = None;
        let mut server_id: Option<uuid::Uuid> = None;
        let mut service_id: Option<uuid::Uuid> = None;

        // Resolve server if provided — this gives us site and client context too
        if let Some(slug) = &p.server_slug {
            if let Ok(Some(server)) =
                crate::repo::server_repo::get_server_by_slug(&self.pool, slug).await
            {
                server_id = Some(server.id);
                site_id = Some(server.site_id);
                awareness.server = serde_json::to_value(&server).ok();

                // Get services for this server
                if let Ok(services) =
                    crate::repo::service_repo::get_services_for_server(&self.pool, server.id).await
                {
                    awareness.services = services
                        .iter()
                        .filter_map(|s| serde_json::to_value(s).ok())
                        .collect();
                }

                // Get site
                if let Ok(Some(site)) =
                    crate::repo::site_repo::get_site(&self.pool, server.site_id).await
                {
                    client_id = Some(site.client_id);
                    awareness.site = serde_json::to_value(&site).ok();

                    // Get client from site
                    if let Ok(Some(client)) =
                        crate::repo::client_repo::get_client(&self.pool, site.client_id).await
                    {
                        awareness.client = serde_json::to_value(&client).ok();
                    }
                }

                // Get networks for this site
                if let Some(sid) = site_id {
                    if let Ok(networks) =
                        crate::repo::network_repo::list_networks(&self.pool, Some(sid)).await
                    {
                        awareness.networks = networks
                            .iter()
                            .filter_map(|n| serde_json::to_value(n).ok())
                            .collect();
                    }
                }

                // Get runbooks linked to this server
                if let Ok(runbooks) = crate::repo::runbook_repo::list_runbooks(
                    &self.pool,
                    None,
                    None,
                    Some(server.id),
                    None,
                    None,
                )
                .await
                {
                    awareness.relevant_runbooks = runbooks
                        .iter()
                        .filter_map(|r| serde_json::to_value(r).ok())
                        .collect();
                }
            } else {
                return Ok(not_found("Server", slug));
            }
        }

        // Resolve service if provided
        if let Some(slug) = &p.service_slug {
            if let Ok(Some(svc)) =
                crate::repo::service_repo::get_service_by_slug(&self.pool, slug).await
            {
                service_id = Some(svc.id);

                // Add service to list if not already present from server lookup
                if awareness.services.is_empty() {
                    awareness.services =
                        vec![serde_json::to_value(&svc).unwrap_or(serde_json::Value::Null)];
                }

                // Get servers running this service
                if let Ok(servers) =
                    crate::repo::service_repo::get_servers_for_service(&self.pool, svc.id).await
                {
                    // If we don't have a server yet, use the first one for context
                    if awareness.server.is_none() {
                        if let Some(first_server) = servers.first() {
                            server_id = Some(first_server.id);
                            #[allow(unused_assignments)]
                            {
                                site_id = Some(first_server.site_id);
                            }
                            awareness.server = serde_json::to_value(first_server).ok();

                            if let Ok(Some(site)) =
                                crate::repo::site_repo::get_site(&self.pool, first_server.site_id)
                                    .await
                            {
                                client_id = Some(site.client_id);
                                awareness.site = serde_json::to_value(&site).ok();
                            }
                        }
                    }
                }

                // Get runbooks linked to this service (merge with existing)
                if let Ok(runbooks) = crate::repo::runbook_repo::list_runbooks(
                    &self.pool,
                    None,
                    Some(svc.id),
                    None,
                    None,
                    None,
                )
                .await
                {
                    for rb in &runbooks {
                        if let Ok(val) = serde_json::to_value(rb) {
                            if !awareness.relevant_runbooks.contains(&val) {
                                awareness.relevant_runbooks.push(val);
                            }
                        }
                    }
                }
            } else {
                return Ok(not_found("Service", slug));
            }
        }

        // Resolve client if provided (may already be set from server/service lookup)
        if let Some(slug) = &p.client_slug {
            if let Ok(Some(client)) =
                crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await
            {
                client_id = Some(client.id);
                awareness.client = serde_json::to_value(&client).ok();
            } else {
                return Ok(not_found("Client", slug));
            }
        }

        // Get vendors for client
        if let Some(cid) = client_id {
            if let Ok(vendors) =
                crate::repo::vendor_repo::get_vendors_for_client(&self.pool, cid).await
            {
                awareness.vendors = vendors
                    .iter()
                    .filter_map(|v| serde_json::to_value(v).ok())
                    .collect();
            }

            // Get recent incidents for this client
            let incidents: Vec<Incident> = sqlx::query_as::<_, Incident>(
                "SELECT * FROM incidents WHERE client_id = $1 ORDER BY reported_at DESC LIMIT 10",
            )
            .bind(cid)
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

            awareness.recent_incidents = incidents
                .iter()
                .filter_map(|i| serde_json::to_value(i).ok())
                .collect();

            // Get knowledge for this client
            if let Ok(entries) =
                crate::repo::knowledge_repo::list_knowledge(&self.pool, None, Some(cid)).await
            {
                awareness.knowledge = entries
                    .iter()
                    .filter_map(|k| serde_json::to_value(k).ok())
                    .collect();
            }
        }

        // If we have a server, also get incidents linked to that server
        if let Some(srv_id) = server_id {
            let server_incidents: Vec<Incident> = sqlx::query_as::<_, Incident>(
                "SELECT i.* FROM incidents i \
                 JOIN incident_servers isv ON i.id = isv.incident_id \
                 WHERE isv.server_id = $1 \
                 ORDER BY i.reported_at DESC LIMIT 10",
            )
            .bind(srv_id)
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

            // Merge with existing incidents (avoid duplicates by ID)
            for inc in &server_incidents {
                if let Ok(val) = serde_json::to_value(inc) {
                    if !awareness
                        .recent_incidents
                        .iter()
                        .any(|existing| existing.get("id") == val.get("id"))
                    {
                        awareness.recent_incidents.push(val);
                    }
                }
            }
        }

        // If we have a service, also get incidents linked to that service
        if let Some(svc_id) = service_id {
            let service_incidents: Vec<Incident> = sqlx::query_as::<_, Incident>(
                "SELECT i.* FROM incidents i \
                 JOIN incident_services iss ON i.id = iss.incident_id \
                 WHERE iss.service_id = $1 \
                 ORDER BY i.reported_at DESC LIMIT 10",
            )
            .bind(svc_id)
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default();

            for inc in &service_incidents {
                if let Ok(val) = serde_json::to_value(inc) {
                    if !awareness
                        .recent_incidents
                        .iter()
                        .any(|existing| existing.get("id") == val.get("id"))
                    {
                        awareness.recent_incidents.push(val);
                    }
                }
            }
        }

        // Get pending handoffs
        let handoffs: Vec<Handoff> = sqlx::query_as::<_, Handoff>(
            "SELECT * FROM handoffs WHERE status = 'pending' ORDER BY created_at DESC LIMIT 10",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        awareness.pending_handoffs = handoffs
            .iter()
            .filter_map(|h| serde_json::to_value(h).ok())
            .collect();

        // Also add general knowledge (not client-specific)
        if let Ok(general_knowledge) =
            crate::repo::knowledge_repo::list_knowledge(&self.pool, None, None).await
        {
            for entry in &general_knowledge {
                if let Ok(val) = serde_json::to_value(entry) {
                    if !awareness
                        .knowledge
                        .iter()
                        .any(|existing| existing.get("id") == val.get("id"))
                    {
                        awareness.knowledge.push(val);
                    }
                }
            }
        }

        // Semantic enrichment: find related runbooks/knowledge beyond explicit links
        if self.embedding_client.is_some() {
            // Build context string from resolved entities
            let mut context_parts = Vec::new();
            if let Some(ref srv) = awareness.server {
                if let Some(hostname) = srv.get("hostname").and_then(|v| v.as_str()) {
                    context_parts.push(hostname.to_string());
                }
                if let Some(os) = srv.get("os").and_then(|v| v.as_str()) {
                    context_parts.push(os.to_string());
                }
            }
            for svc in &awareness.services {
                if let Some(name) = svc.get("name").and_then(|v| v.as_str()) {
                    context_parts.push(name.to_string());
                }
            }
            if let Some(ref client) = awareness.client {
                if let Some(name) = client.get("name").and_then(|v| v.as_str()) {
                    context_parts.push(name.to_string());
                }
            }

            if !context_parts.is_empty() {
                let context_query = context_parts.join(" ");
                if let Some(emb) = self.get_query_embedding(&context_query).await {
                    // Find semantically related runbooks
                    if let Ok(related_runbooks) =
                        crate::repo::embedding_repo::vector_search_runbooks(&self.pool, &emb, 5)
                            .await
                    {
                        for rb in &related_runbooks {
                            if let Ok(val) = serde_json::to_value(rb) {
                                if !awareness
                                    .relevant_runbooks
                                    .iter()
                                    .any(|existing| existing.get("id") == val.get("id"))
                                {
                                    awareness.relevant_runbooks.push(val);
                                }
                            }
                        }
                    }
                    // Find semantically related knowledge
                    if let Ok(related_knowledge) =
                        crate::repo::embedding_repo::vector_search_knowledge(&self.pool, &emb, 5)
                            .await
                    {
                        for k in &related_knowledge {
                            if let Ok(val) = serde_json::to_value(k) {
                                if !awareness
                                    .knowledge
                                    .iter()
                                    .any(|existing| existing.get("id") == val.get("id"))
                                {
                                    awareness.knowledge.push(val);
                                }
                            }
                        }
                    }
                }
            }
        }

        // ── Cross-client scope gate for runbooks and knowledge ──
        {
            let client_lookup = self.build_client_lookup().await;

            let rb_filtered = filter_cross_client(
                std::mem::take(&mut awareness.relevant_runbooks),
                "runbook",
                client_id,
                acknowledge,
                &client_lookup,
            );
            awareness.relevant_runbooks = rb_filtered.allowed;
            awareness
                .cross_client_withheld
                .extend(rb_filtered.withheld_notices);
            self.log_audit_entries(
                "get_situational_awareness",
                client_id,
                "runbook",
                &rb_filtered.audit_entries,
            )
            .await;

            let kn_filtered = filter_cross_client(
                std::mem::take(&mut awareness.knowledge),
                "knowledge",
                client_id,
                acknowledge,
                &client_lookup,
            );
            awareness.knowledge = kn_filtered.allowed;
            awareness
                .cross_client_withheld
                .extend(kn_filtered.withheld_notices);
            self.log_audit_entries(
                "get_situational_awareness",
                client_id,
                "knowledge",
                &kn_filtered.audit_entries,
            )
            .await;
        }

        // Fetch live monitoring data for linked servers/services
        if let Some(ref kuma_config) = self.kuma_config {
            if let Ok(metrics) = crate::metrics::fetch_metrics(kuma_config).await {
                // Get monitor mappings for this server and its services
                let mut monitor_names: std::collections::HashSet<String> =
                    std::collections::HashSet::new();

                if let Some(srv_id) = server_id {
                    if let Ok(monitors) =
                        crate::repo::monitor_repo::get_monitors_for_server(&self.pool, srv_id).await
                    {
                        for m in &monitors {
                            monitor_names.insert(m.monitor_name.clone());
                        }
                    }
                }

                if let Some(svc_id) = service_id {
                    if let Ok(monitors) =
                        crate::repo::monitor_repo::get_monitors_for_service(&self.pool, svc_id)
                            .await
                    {
                        for m in &monitors {
                            monitor_names.insert(m.monitor_name.clone());
                        }
                    }
                }

                // Match metrics to mapped monitors
                for status in &metrics.monitors {
                    if monitor_names.contains(&status.name) {
                        if let Ok(val) = serde_json::to_value(status) {
                            awareness.monitoring.push(val);
                        }
                    }
                }
            }
        }

        // Zammad linked tickets for this server/service
        if self.zammad_config.is_some() {
            if let Some(srv_id) = server_id {
                if let Ok(links) =
                    crate::repo::ticket_link_repo::get_links_for_server(&self.pool, srv_id).await
                {
                    for link in &links {
                        if let Ok(val) = serde_json::to_value(link) {
                            awareness.linked_tickets.push(val);
                        }
                    }
                }
            }
            if let Some(svc_id) = service_id {
                if let Ok(links) =
                    crate::repo::ticket_link_repo::get_links_for_service(&self.pool, svc_id).await
                {
                    for link in &links {
                        if let Ok(val) = serde_json::to_value(link) {
                            awareness.linked_tickets.push(val);
                        }
                    }
                }
            }
        }

        Ok(json_result(&awareness))
    }

    #[tool(
        name = "get_client_overview",
        description = "Get a full client briefing: all sites, servers, services, networks, vendors, recent incidents, and pending handoffs"
    )]
    async fn get_client_overview(
        &self,
        params: Parameters<context::GetClientOverviewParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let client =
            match crate::repo::client_repo::get_client_by_slug(&self.pool, &p.client_slug).await {
                Ok(Some(c)) => c,
                Ok(None) => return Ok(not_found("Client", &p.client_slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        let sites = crate::repo::site_repo::list_sites(&self.pool, Some(client.id))
            .await
            .unwrap_or_default();

        let servers =
            crate::repo::server_repo::list_servers(&self.pool, Some(client.id), None, None, None)
                .await
                .unwrap_or_default();

        // Collect all service IDs from all servers
        let mut all_services = Vec::new();
        let mut seen_service_ids = std::collections::HashSet::new();
        for server in &servers {
            if let Ok(svcs) =
                crate::repo::service_repo::get_services_for_server(&self.pool, server.id).await
            {
                for svc in svcs {
                    if seen_service_ids.insert(svc.id) {
                        all_services.push(svc);
                    }
                }
            }
        }

        // Get networks for all sites
        let mut all_networks = Vec::new();
        for site in &sites {
            if let Ok(nets) =
                crate::repo::network_repo::list_networks(&self.pool, Some(site.id)).await
            {
                all_networks.extend(nets);
            }
        }

        let vendors = crate::repo::vendor_repo::get_vendors_for_client(&self.pool, client.id)
            .await
            .unwrap_or_default();

        let recent_incidents: Vec<Incident> = sqlx::query_as::<_, Incident>(
            "SELECT * FROM incidents WHERE client_id = $1 ORDER BY reported_at DESC LIMIT 10",
        )
        .bind(client.id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let pending_handoffs: Vec<Handoff> = sqlx::query_as::<_, Handoff>(
            "SELECT * FROM handoffs WHERE status = 'pending' ORDER BY created_at DESC LIMIT 10",
        )
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let mut overview = context::ClientOverview {
            client: serde_json::to_value(&client).unwrap_or(serde_json::Value::Null),
            sites: sites
                .iter()
                .filter_map(|s| serde_json::to_value(s).ok())
                .collect(),
            servers: servers
                .iter()
                .filter_map(|s| serde_json::to_value(s).ok())
                .collect(),
            services: all_services
                .iter()
                .filter_map(|s| serde_json::to_value(s).ok())
                .collect(),
            networks: all_networks
                .iter()
                .filter_map(|n| serde_json::to_value(n).ok())
                .collect(),
            vendors: vendors
                .iter()
                .filter_map(|v| serde_json::to_value(v).ok())
                .collect(),
            recent_incidents: recent_incidents
                .iter()
                .filter_map(|i| serde_json::to_value(i).ok())
                .collect(),
            pending_handoffs: pending_handoffs
                .iter()
                .filter_map(|h| serde_json::to_value(h).ok())
                .collect(),
            recent_tickets: Vec::new(),
        };

        // Fetch recent Zammad tickets for this client
        if let Some(ref zammad) = self.zammad_config {
            if let Some(org_id) = client.zammad_org_id {
                let query = format!("organization.id:{org_id}");
                match crate::zammad::search_tickets(zammad, &query, 5).await {
                    Ok(tickets) => {
                        overview.recent_tickets = tickets
                            .iter()
                            .filter_map(|t| serde_json::to_value(t).ok())
                            .collect();
                    }
                    Err(e) => {
                        tracing::warn!("Failed to fetch Zammad tickets for client overview: {e}");
                    }
                }
            }
        }

        Ok(json_result(&overview))
    }

    #[tool(
        name = "get_server_context",
        description = "Get everything about a specific server: details, services, site, networks, \
        recent incidents for this server, related runbooks, and vendor contacts"
    )]
    async fn get_server_context(
        &self,
        params: Parameters<context::GetServerContextParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

        let server =
            match crate::repo::server_repo::get_server_by_slug(&self.pool, &p.server_slug).await {
                Ok(Some(s)) => s,
                Ok(None) => return Ok(not_found("Server", &p.server_slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        let services = crate::repo::service_repo::get_services_for_server(&self.pool, server.id)
            .await
            .unwrap_or_default();

        let site = crate::repo::site_repo::get_site(&self.pool, server.site_id)
            .await
            .ok()
            .flatten();

        let networks = crate::repo::network_repo::list_networks(&self.pool, Some(server.site_id))
            .await
            .unwrap_or_default();

        // Get client for vendor lookup
        let client_id = site.as_ref().map(|s| s.client_id);
        let client = if let Some(cid) = client_id {
            crate::repo::client_repo::get_client(&self.pool, cid)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        let vendors = if let Some(cid) = client_id {
            crate::repo::vendor_repo::get_vendors_for_client(&self.pool, cid)
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Get incidents linked to this server
        let incidents: Vec<Incident> = sqlx::query_as::<_, Incident>(
            "SELECT i.* FROM incidents i \
             JOIN incident_servers isv ON i.id = isv.incident_id \
             WHERE isv.server_id = $1 \
             ORDER BY i.reported_at DESC LIMIT 10",
        )
        .bind(server.id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        // Also get client-level incidents
        let client_incidents: Vec<Incident> = if let Some(cid) = client_id {
            sqlx::query_as::<_, Incident>(
                "SELECT * FROM incidents WHERE client_id = $1 ORDER BY reported_at DESC LIMIT 10",
            )
            .bind(cid)
            .fetch_all(&self.pool)
            .await
            .unwrap_or_default()
        } else {
            Vec::new()
        };

        // Merge incidents, dedup by id
        let mut all_incidents: Vec<serde_json::Value> = Vec::new();
        let mut seen_ids = std::collections::HashSet::new();
        for inc in incidents.iter().chain(client_incidents.iter()) {
            if seen_ids.insert(inc.id) {
                if let Ok(val) = serde_json::to_value(inc) {
                    all_incidents.push(val);
                }
            }
        }

        // Get runbooks linked to this server
        let runbooks = crate::repo::runbook_repo::list_runbooks(
            &self.pool,
            None,
            None,
            Some(server.id),
            None,
            None,
        )
        .await
        .unwrap_or_default();

        // Also get runbooks linked to any of this server's services
        let mut all_runbooks: Vec<serde_json::Value> = runbooks
            .iter()
            .filter_map(|r| serde_json::to_value(r).ok())
            .collect();
        let mut seen_runbook_ids: std::collections::HashSet<uuid::Uuid> =
            runbooks.iter().map(|r| r.id).collect();

        for svc in &services {
            if let Ok(svc_runbooks) = crate::repo::runbook_repo::list_runbooks(
                &self.pool,
                None,
                Some(svc.id),
                None,
                None,
                None,
            )
            .await
            {
                for rb in &svc_runbooks {
                    if seen_runbook_ids.insert(rb.id) {
                        if let Ok(val) = serde_json::to_value(rb) {
                            all_runbooks.push(val);
                        }
                    }
                }
            }
        }

        // Get knowledge entries for this client
        let mut all_knowledge: Vec<serde_json::Value> = if let Some(cid) = client_id {
            crate::repo::knowledge_repo::list_knowledge(&self.pool, None, Some(cid))
                .await
                .unwrap_or_default()
                .iter()
                .filter_map(|k| serde_json::to_value(k).ok())
                .collect()
        } else {
            Vec::new()
        };
        let mut seen_knowledge_ids: std::collections::HashSet<uuid::Uuid> = all_knowledge
            .iter()
            .filter_map(|v| {
                v.get("id")
                    .and_then(|id| id.as_str())
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
            })
            .collect();

        // Semantic enrichment: find related runbooks/knowledge beyond explicit links
        if self.embedding_client.is_some() {
            let mut context_parts = vec![server.hostname.clone()];
            if let Some(ref os) = server.os {
                context_parts.push(os.clone());
            }
            for svc in &services {
                context_parts.push(svc.name.clone());
            }
            let context_query = context_parts.join(" ");
            if let Some(emb) = self.get_query_embedding(&context_query).await {
                if let Ok(related_runbooks) =
                    crate::repo::embedding_repo::vector_search_runbooks(&self.pool, &emb, 5).await
                {
                    for rb in &related_runbooks {
                        if seen_runbook_ids.insert(rb.id) {
                            if let Ok(val) = serde_json::to_value(rb) {
                                all_runbooks.push(val);
                            }
                        }
                    }
                }
                if let Ok(related_knowledge) =
                    crate::repo::embedding_repo::vector_search_knowledge(&self.pool, &emb, 5).await
                {
                    for k in &related_knowledge {
                        if seen_knowledge_ids.insert(k.id) {
                            if let Ok(val) = serde_json::to_value(k) {
                                all_knowledge.push(val);
                            }
                        }
                    }
                }
            }
        }

        // ── Cross-client scope gate for runbooks and knowledge ──
        let mut cross_client_withheld: Vec<serde_json::Value> = Vec::new();
        {
            let client_lookup = self.build_client_lookup().await;

            let rb_filtered = filter_cross_client(
                std::mem::take(&mut all_runbooks),
                "runbook",
                client_id,
                acknowledge,
                &client_lookup,
            );
            all_runbooks = rb_filtered.allowed;
            cross_client_withheld.extend(rb_filtered.withheld_notices);
            self.log_audit_entries(
                "get_server_context",
                client_id,
                "runbook",
                &rb_filtered.audit_entries,
            )
            .await;

            let kn_filtered = filter_cross_client(
                std::mem::take(&mut all_knowledge),
                "knowledge",
                client_id,
                acknowledge,
                &client_lookup,
            );
            all_knowledge = kn_filtered.allowed;
            cross_client_withheld.extend(kn_filtered.withheld_notices);
            self.log_audit_entries(
                "get_server_context",
                client_id,
                "knowledge",
                &kn_filtered.audit_entries,
            )
            .await;
        }

        // Fetch live monitoring data for this server and its services
        let mut monitoring: Vec<serde_json::Value> = Vec::new();
        if let Some(ref kuma_config) = self.kuma_config {
            if let Ok(metrics) = crate::metrics::fetch_metrics(kuma_config).await {
                let mut monitor_names: std::collections::HashSet<String> =
                    std::collections::HashSet::new();

                if let Ok(monitors) =
                    crate::repo::monitor_repo::get_monitors_for_server(&self.pool, server.id).await
                {
                    for m in &monitors {
                        monitor_names.insert(m.monitor_name.clone());
                    }
                }

                for svc in &services {
                    if let Ok(monitors) =
                        crate::repo::monitor_repo::get_monitors_for_service(&self.pool, svc.id)
                            .await
                    {
                        for m in &monitors {
                            monitor_names.insert(m.monitor_name.clone());
                        }
                    }
                }

                for status in &metrics.monitors {
                    if monitor_names.contains(&status.name) {
                        if let Ok(val) = serde_json::to_value(status) {
                            monitoring.push(val);
                        }
                    }
                }
            }
        }

        // Zammad linked tickets for this server
        let mut linked_tickets: Vec<serde_json::Value> = Vec::new();
        if self.zammad_config.is_some() {
            if let Ok(links) =
                crate::repo::ticket_link_repo::get_links_for_server(&self.pool, server.id).await
            {
                for link in &links {
                    if let Ok(val) = serde_json::to_value(link) {
                        linked_tickets.push(val);
                    }
                }
            }
        }

        let mut result = serde_json::json!({
            "server": server,
            "services": services,
            "site": site,
            "client": client,
            "networks": networks,
            "vendors": vendors,
            "recent_incidents": all_incidents,
            "runbooks": all_runbooks,
            "knowledge": all_knowledge,
            "monitoring": monitoring,
        });
        if !linked_tickets.is_empty() {
            result["linked_tickets"] = serde_json::json!(linked_tickets);
        }
        if !cross_client_withheld.is_empty() {
            result["cross_client_withheld"] = serde_json::json!(cross_client_withheld);
        }

        Ok(json_result(&result))
    }

    // ===== INCIDENT TOOLS =====

    #[tool(
        name = "create_incident",
        description = "Create a new incident. Optionally link to affected servers and services immediately."
    )]
    async fn create_incident(
        &self,
        params: Parameters<incidents::CreateIncidentParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let severity = p.severity.as_deref().unwrap_or("medium");

        if let Err(msg) = crate::validation::validate_required(
            severity,
            "severity",
            crate::validation::INCIDENT_SEVERITIES,
        ) {
            return Ok(error_result(&msg));
        }

        // Resolve client_slug
        let client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        let incident = match crate::repo::incident_repo::create_incident(
            &self.pool,
            &p.title,
            severity,
            client_id,
            p.symptoms.as_deref(),
            p.notes.as_deref(),
        )
        .await
        {
            Ok(i) => i,
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        };

        // Link servers if provided
        if let Some(slugs) = &p.server_slugs {
            for slug in slugs {
                if let Ok(Some(server)) =
                    crate::repo::server_repo::get_server_by_slug(&self.pool, slug).await
                {
                    let _ = crate::repo::incident_repo::link_incident_server(
                        &self.pool,
                        incident.id,
                        server.id,
                    )
                    .await;
                }
            }
        }

        // Link services if provided
        if let Some(slugs) = &p.service_slugs {
            for slug in slugs {
                if let Ok(Some(service)) =
                    crate::repo::service_repo::get_service_by_slug(&self.pool, slug).await
                {
                    let _ = crate::repo::incident_repo::link_incident_service(
                        &self.pool,
                        incident.id,
                        service.id,
                    )
                    .await;
                }
            }
        }

        let text = crate::embeddings::prepare_incident_text(&incident);
        self.embed_and_store("incidents", incident.id, &text).await;

        Ok(json_result(&incident))
    }

    #[tool(
        name = "update_incident",
        description = "Update an incident. Set status to 'resolved' to auto-calculate resolved_at and TTR. \
        Use for post-mortems: root_cause, resolution, prevention fields."
    )]
    async fn update_incident(
        &self,
        params: Parameters<incidents::UpdateIncidentParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let id = match uuid::Uuid::parse_str(&p.id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.id))),
        };

        if let Err(msg) = crate::validation::validate_option(
            p.status.as_deref(),
            "status",
            crate::validation::INCIDENT_STATUSES,
        ) {
            return Ok(error_result(&msg));
        }
        if let Err(msg) = crate::validation::validate_option(
            p.severity.as_deref(),
            "severity",
            crate::validation::INCIDENT_SEVERITIES,
        ) {
            return Ok(error_result(&msg));
        }

        match crate::repo::incident_repo::update_incident(
            &self.pool,
            id,
            p.title.as_deref(),
            p.status.as_deref(),
            p.severity.as_deref(),
            p.symptoms.as_deref(),
            p.root_cause.as_deref(),
            p.resolution.as_deref(),
            p.prevention.as_deref(),
            p.notes.as_deref(),
        )
        .await
        {
            Ok(incident) => {
                let text = crate::embeddings::prepare_incident_text(&incident);
                self.embed_and_store("incidents", incident.id, &text).await;
                Ok(json_result(&incident))
            }
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "get_incident",
        description = "Get full details of an incident by ID, including linked servers, services, runbooks, and vendors"
    )]
    async fn get_incident(
        &self,
        params: Parameters<incidents::GetIncidentParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let id = match uuid::Uuid::parse_str(&p.id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.id))),
        };

        let incident = match crate::repo::incident_repo::get_incident(&self.pool, id).await {
            Ok(Some(i)) => i,
            Ok(None) => return Ok(not_found("Incident", &p.id)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        };

        // Get linked entities
        let linked_servers: Vec<crate::models::server::Server> = sqlx::query_as(
            "SELECT s.* FROM servers s JOIN incident_servers isv ON s.id = isv.server_id WHERE isv.incident_id = $1",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let linked_services: Vec<crate::models::service::Service> = sqlx::query_as(
            "SELECT s.* FROM services s JOIN incident_services iss ON s.id = iss.service_id WHERE iss.incident_id = $1",
        )
        .bind(id)
        .fetch_all(&self.pool)
        .await
        .unwrap_or_default();

        let result = serde_json::json!({
            "incident": incident,
            "linked_servers": linked_servers,
            "linked_services": linked_services,
        });

        Ok(json_result(&result))
    }

    #[tool(
        name = "list_incidents",
        description = "List incidents with optional filters by client, status, and severity"
    )]
    async fn list_incidents(
        &self,
        params: Parameters<incidents::ListIncidentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20);

        // Validate filters
        if let Err(msg) = crate::validation::validate_option(
            p.status.as_deref(),
            "status",
            crate::validation::INCIDENT_STATUSES,
        ) {
            return Ok(error_result(&msg));
        }
        if let Err(msg) = crate::validation::validate_option(
            p.severity.as_deref(),
            "severity",
            crate::validation::INCIDENT_SEVERITIES,
        ) {
            return Ok(error_result(&msg));
        }

        // Resolve client_slug
        let client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        match crate::repo::incident_repo::list_incidents(
            &self.pool,
            client_id,
            p.status.as_deref(),
            p.severity.as_deref(),
            limit,
        )
        .await
        {
            Ok(incidents) => Ok(json_result(&incidents)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "search_incidents",
        description = "Search across incident titles, symptoms, root causes, resolutions, and notes. \
        Supports mode: 'fts' (default, keyword match), 'semantic' (AI vector similarity), \
        or 'hybrid' (combined FTS + vector via RRF ranking)"
    )]
    async fn search_incidents(
        &self,
        params: Parameters<incidents::SearchIncidentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let mode = p.mode.as_deref().unwrap_or("fts");
        if let Err(msg) =
            crate::validation::validate_required(mode, "mode", crate::validation::SEARCH_MODES)
        {
            return Ok(error_result(&msg));
        }
        let result = match mode {
            "semantic" => {
                let Some(emb) = self.get_query_embedding(&p.query).await else {
                    return Ok(error_result(
                        "Semantic search unavailable (OPENAI_API_KEY not set)",
                    ));
                };
                crate::repo::embedding_repo::vector_search_incidents(&self.pool, &emb, 20).await
            }
            "hybrid" => {
                let emb = self.get_query_embedding(&p.query).await;
                crate::repo::embedding_repo::hybrid_search_incidents(
                    &self.pool,
                    &p.query,
                    emb.as_deref(),
                    20,
                )
                .await
            }
            _ => crate::repo::incident_repo::search_incidents(&self.pool, &p.query).await,
        };
        match result {
            Ok(incidents) => Ok(json_result(&incidents)),
            Err(e) => Ok(error_result(&format!("Search error: {e}"))),
        }
    }

    #[tool(
        name = "link_incident",
        description = "Link an incident to servers, services, runbooks, and/or vendors. \
        Runbook links include usage tracking: 'followed', 'not-applicable', or 'not-followed'."
    )]
    async fn link_incident(
        &self,
        params: Parameters<incidents::LinkIncidentParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let incident_id = match uuid::Uuid::parse_str(&p.incident_id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.incident_id))),
        };

        // Verify incident exists
        match crate::repo::incident_repo::get_incident(&self.pool, incident_id).await {
            Ok(Some(_)) => {}
            Ok(None) => return Ok(not_found("Incident", &p.incident_id)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        }

        let mut linked = Vec::new();

        // Link servers
        if let Some(slugs) = &p.server_slugs {
            for slug in slugs {
                match crate::repo::server_repo::get_server_by_slug(&self.pool, slug).await {
                    Ok(Some(server)) => {
                        if let Err(e) = crate::repo::incident_repo::link_incident_server(
                            &self.pool,
                            incident_id,
                            server.id,
                        )
                        .await
                        {
                            return Ok(error_result(&format!(
                                "Failed to link server '{slug}': {e}"
                            )));
                        }
                        linked.push(format!("server:{slug}"));
                    }
                    Ok(None) => return Ok(not_found("Server", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
        }

        // Link services
        if let Some(slugs) = &p.service_slugs {
            for slug in slugs {
                match crate::repo::service_repo::get_service_by_slug(&self.pool, slug).await {
                    Ok(Some(service)) => {
                        if let Err(e) = crate::repo::incident_repo::link_incident_service(
                            &self.pool,
                            incident_id,
                            service.id,
                        )
                        .await
                        {
                            return Ok(error_result(&format!(
                                "Failed to link service '{slug}': {e}"
                            )));
                        }
                        linked.push(format!("service:{slug}"));
                    }
                    Ok(None) => return Ok(not_found("Service", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
        }

        // Link runbooks
        if let Some(rb_links) = &p.runbook_links {
            for rb_link in rb_links {
                let usage = rb_link.usage.as_deref().unwrap_or("followed");
                if let Err(msg) = crate::validation::validate_required(
                    usage,
                    "runbook usage",
                    crate::validation::RUNBOOK_USAGES,
                ) {
                    return Ok(error_result(&msg));
                }
                match crate::repo::runbook_repo::get_runbook_by_slug(&self.pool, &rb_link.slug)
                    .await
                {
                    Ok(Some(runbook)) => {
                        if let Err(e) = crate::repo::incident_repo::link_incident_runbook(
                            &self.pool,
                            incident_id,
                            runbook.id,
                            usage,
                        )
                        .await
                        {
                            return Ok(error_result(&format!(
                                "Failed to link runbook '{}': {e}",
                                rb_link.slug
                            )));
                        }
                        linked.push(format!("runbook:{}", rb_link.slug));
                    }
                    Ok(None) => return Ok(not_found("Runbook", &rb_link.slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
        }

        // Link vendors
        if let Some(names) = &p.vendor_names {
            for name in names {
                match crate::repo::vendor_repo::get_vendor_by_name(&self.pool, name).await {
                    Ok(Some(vendor)) => {
                        if let Err(e) = crate::repo::incident_repo::link_incident_vendor(
                            &self.pool,
                            incident_id,
                            vendor.id,
                        )
                        .await
                        {
                            return Ok(error_result(&format!(
                                "Failed to link vendor '{name}': {e}"
                            )));
                        }
                        linked.push(format!("vendor:{name}"));
                    }
                    Ok(None) => return Ok(not_found("Vendor", name)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
        }

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Linked to incident {}: {}",
            p.incident_id,
            linked.join(", ")
        ))]))
    }

    // ===== SESSION TOOLS =====

    #[tool(
        name = "start_session",
        description = "Start a new work session on a machine. Returns session ID for handoff tracking."
    )]
    async fn start_session(
        &self,
        params: Parameters<coordination::StartSessionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::session_repo::start_session(
            &self.pool,
            &p.machine_id,
            &p.machine_hostname,
        )
        .await
        {
            Ok(session) => Ok(json_result(&session)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "end_session",
        description = "End a work session with an optional summary of what was accomplished"
    )]
    async fn end_session(
        &self,
        params: Parameters<coordination::EndSessionParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let id = match uuid::Uuid::parse_str(&p.session_id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.session_id))),
        };

        match crate::repo::session_repo::end_session(&self.pool, id, p.summary.as_deref()).await {
            Ok(session) => Ok(json_result(&session)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "list_sessions",
        description = "List work sessions, optionally filtered by machine and active status"
    )]
    async fn list_sessions(
        &self,
        params: Parameters<coordination::ListSessionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20);
        let active_only = p.active_only.unwrap_or(false);

        match crate::repo::session_repo::list_sessions(
            &self.pool,
            p.machine_id.as_deref(),
            active_only,
            limit,
        )
        .await
        {
            Ok(sessions) => Ok(json_result(&sessions)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    // ===== HANDOFF TOOLS =====

    #[tool(
        name = "create_handoff",
        description = "Create a handoff task for another machine/session to pick up. \
        Use for cross-machine coordination: tasks that need to continue on a different machine."
    )]
    async fn create_handoff(
        &self,
        params: Parameters<coordination::CreateHandoffParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let priority = p.priority.as_deref().unwrap_or("normal");

        if let Err(msg) = crate::validation::validate_required(
            priority,
            "priority",
            crate::validation::HANDOFF_PRIORITIES,
        ) {
            return Ok(error_result(&msg));
        }

        // Resolve optional session ID
        let from_session_id = match &p.from_session_id {
            Some(id_str) => match uuid::Uuid::parse_str(id_str) {
                Ok(id) => Some(id),
                Err(_) => return Ok(error_result(&format!("Invalid session UUID: {id_str}"))),
            },
            None => None,
        };

        match crate::repo::handoff_repo::create_handoff(
            &self.pool,
            from_session_id,
            &p.from_machine,
            p.to_machine.as_deref(),
            priority,
            &p.title,
            &p.body,
            p.context.as_ref(),
        )
        .await
        {
            Ok(handoff) => {
                let text = crate::embeddings::prepare_handoff_text(&handoff);
                self.embed_and_store("handoffs", handoff.id, &text).await;
                Ok(json_result(&handoff))
            }
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "accept_handoff",
        description = "Accept a pending handoff, marking it as in-progress on your machine"
    )]
    async fn accept_handoff(
        &self,
        params: Parameters<coordination::UpdateHandoffStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let id = match uuid::Uuid::parse_str(&p.handoff_id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.handoff_id))),
        };

        // Verify it's pending
        match crate::repo::handoff_repo::get_handoff(&self.pool, id).await {
            Ok(Some(h)) if h.status == "pending" => {}
            Ok(Some(h)) => {
                return Ok(error_result(&format!(
                    "Handoff is already '{}', cannot accept",
                    h.status
                )))
            }
            Ok(None) => return Ok(not_found("Handoff", &p.handoff_id)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        }

        match crate::repo::handoff_repo::update_handoff_status(&self.pool, id, "accepted").await {
            Ok(handoff) => Ok(json_result(&handoff)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(name = "complete_handoff", description = "Mark a handoff as completed")]
    async fn complete_handoff(
        &self,
        params: Parameters<coordination::UpdateHandoffStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        let id = match uuid::Uuid::parse_str(&p.handoff_id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.handoff_id))),
        };

        // Verify it exists and is not already completed
        match crate::repo::handoff_repo::get_handoff(&self.pool, id).await {
            Ok(Some(h)) if h.status == "completed" => {
                return Ok(error_result("Handoff is already completed"))
            }
            Ok(Some(_)) => {}
            Ok(None) => return Ok(not_found("Handoff", &p.handoff_id)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        }

        match crate::repo::handoff_repo::update_handoff_status(&self.pool, id, "completed").await {
            Ok(handoff) => Ok(json_result(&handoff)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "list_handoffs",
        description = "List handoffs with optional filters. Use status='pending' to see what needs attention."
    )]
    async fn list_handoffs(
        &self,
        params: Parameters<coordination::ListHandoffsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20);

        if let Err(msg) = crate::validation::validate_option(
            p.status.as_deref(),
            "status",
            crate::validation::HANDOFF_STATUSES,
        ) {
            return Ok(error_result(&msg));
        }

        match crate::repo::handoff_repo::list_handoffs(
            &self.pool,
            p.status.as_deref(),
            p.to_machine.as_deref(),
            p.from_machine.as_deref(),
            limit,
        )
        .await
        {
            Ok(handoffs) => Ok(json_result(&handoffs)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "search_handoffs",
        description = "Search across handoff titles and bodies. Supports mode: 'fts' (default, keyword match), \
        'semantic' (AI vector similarity), or 'hybrid' (combined FTS + vector via RRF ranking)"
    )]
    async fn search_handoffs(
        &self,
        params: Parameters<coordination::SearchHandoffsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let mode = p.mode.as_deref().unwrap_or("fts");
        if let Err(msg) =
            crate::validation::validate_required(mode, "mode", crate::validation::SEARCH_MODES)
        {
            return Ok(error_result(&msg));
        }
        let result = match mode {
            "semantic" => {
                let Some(emb) = self.get_query_embedding(&p.query).await else {
                    return Ok(error_result(
                        "Semantic search unavailable (OPENAI_API_KEY not set)",
                    ));
                };
                crate::repo::embedding_repo::vector_search_handoffs(&self.pool, &emb, 20).await
            }
            "hybrid" => {
                let emb = self.get_query_embedding(&p.query).await;
                crate::repo::embedding_repo::hybrid_search_handoffs(
                    &self.pool,
                    &p.query,
                    emb.as_deref(),
                    20,
                )
                .await
            }
            _ => crate::repo::handoff_repo::search_handoffs(&self.pool, &p.query).await,
        };
        match result {
            Ok(handoffs) => Ok(json_result(&handoffs)),
            Err(e) => Ok(error_result(&format!("Search error: {e}"))),
        }
    }

    // ===== SEMANTIC SEARCH TOOLS =====

    #[tool(
        name = "semantic_search",
        description = "AI-powered semantic search across runbooks, knowledge, incidents, and handoffs. \
        Finds conceptually related content even when exact keywords don't match. \
        Uses hybrid ranking (FTS + vector similarity) for best results. \
        Falls back to full-text search if embeddings are unavailable."
    )]
    async fn semantic_search(
        &self,
        params: Parameters<search::SemanticSearchParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let limit = p.limit.unwrap_or(5);
        let tables = p.tables.unwrap_or_else(|| {
            vec![
                "runbooks".to_string(),
                "knowledge".to_string(),
                "incidents".to_string(),
                "handoffs".to_string(),
            ]
        });

        // Resolve optional client_slug for cross-client gate
        let requesting_client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };
        let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

        let query_embedding = self.get_query_embedding(&p.query).await;
        let emb_ref = query_embedding.as_deref();

        let mut results = serde_json::Map::new();
        let client_lookup = self.build_client_lookup().await;
        let mut all_withheld: Vec<serde_json::Value> = Vec::new();

        // Run searches for requested tables — gate runbooks and knowledge
        if tables.iter().any(|t| t == "runbooks") {
            match crate::repo::embedding_repo::hybrid_search_runbooks(
                &self.pool, &p.query, emb_ref, limit,
            )
            .await
            {
                Ok(items) => {
                    let json_items: Vec<serde_json::Value> = items
                        .iter()
                        .filter_map(|r| serde_json::to_value(r).ok())
                        .collect();
                    let filtered = filter_cross_client(
                        json_items,
                        "runbook",
                        requesting_client_id,
                        acknowledge,
                        &client_lookup,
                    );
                    results.insert(
                        "runbooks".to_string(),
                        serde_json::to_value(&filtered.allowed).unwrap_or_default(),
                    );
                    all_withheld.extend(filtered.withheld_notices);
                    self.log_audit_entries(
                        "semantic_search",
                        requesting_client_id,
                        "runbook",
                        &filtered.audit_entries,
                    )
                    .await;
                }
                Err(e) => {
                    results.insert(
                        "runbooks_error".to_string(),
                        serde_json::Value::String(e.to_string()),
                    );
                }
            }
        }
        if tables.iter().any(|t| t == "knowledge") {
            match crate::repo::embedding_repo::hybrid_search_knowledge(
                &self.pool, &p.query, emb_ref, limit,
            )
            .await
            {
                Ok(items) => {
                    let json_items: Vec<serde_json::Value> = items
                        .iter()
                        .filter_map(|k| serde_json::to_value(k).ok())
                        .collect();
                    let filtered = filter_cross_client(
                        json_items,
                        "knowledge",
                        requesting_client_id,
                        acknowledge,
                        &client_lookup,
                    );
                    results.insert(
                        "knowledge".to_string(),
                        serde_json::to_value(&filtered.allowed).unwrap_or_default(),
                    );
                    all_withheld.extend(filtered.withheld_notices);
                    self.log_audit_entries(
                        "semantic_search",
                        requesting_client_id,
                        "knowledge",
                        &filtered.audit_entries,
                    )
                    .await;
                }
                Err(e) => {
                    results.insert(
                        "knowledge_error".to_string(),
                        serde_json::Value::String(e.to_string()),
                    );
                }
            }
        }
        // Incidents and handoffs are NOT gated (factual records, already client-scoped)
        if tables.iter().any(|t| t == "incidents") {
            match crate::repo::embedding_repo::hybrid_search_incidents(
                &self.pool, &p.query, emb_ref, limit,
            )
            .await
            {
                Ok(items) => {
                    results.insert(
                        "incidents".to_string(),
                        serde_json::to_value(&items).unwrap_or_default(),
                    );
                }
                Err(e) => {
                    results.insert(
                        "incidents_error".to_string(),
                        serde_json::Value::String(e.to_string()),
                    );
                }
            }
        }
        if tables.iter().any(|t| t == "handoffs") {
            match crate::repo::embedding_repo::hybrid_search_handoffs(
                &self.pool, &p.query, emb_ref, limit,
            )
            .await
            {
                Ok(items) => {
                    results.insert(
                        "handoffs".to_string(),
                        serde_json::to_value(&items).unwrap_or_default(),
                    );
                }
                Err(e) => {
                    results.insert(
                        "handoffs_error".to_string(),
                        serde_json::Value::String(e.to_string()),
                    );
                }
            }
        }

        if !all_withheld.is_empty() {
            results.insert(
                "cross_client_withheld".to_string(),
                serde_json::to_value(&all_withheld).unwrap_or_default(),
            );
        }

        if query_embedding.is_none() && self.embedding_client.is_some() {
            results.insert(
                "_note".to_string(),
                serde_json::Value::String(
                    "Embedding API call failed — results are FTS-only".to_string(),
                ),
            );
        } else if self.embedding_client.is_none() {
            results.insert(
                "_note".to_string(),
                serde_json::Value::String(
                    "OPENAI_API_KEY not set — results are FTS-only".to_string(),
                ),
            );
        }

        Ok(json_result(&serde_json::Value::Object(results)))
    }

    #[tool(
        name = "backfill_embeddings",
        description = "Generate embeddings for records that don't have them yet. \
        Use after initial setup or if records were created without an API key. \
        Processes in batches to avoid API rate limits."
    )]
    async fn backfill_embeddings(
        &self,
        params: Parameters<search::BackfillEmbeddingsParams>,
    ) -> Result<CallToolResult, McpError> {
        let Some(ref client) = self.embedding_client else {
            return Ok(error_result(
                "OPENAI_API_KEY not set — cannot generate embeddings",
            ));
        };

        let p = params.0;
        let batch_size = p.batch_size.unwrap_or(10);
        let tables: Vec<&str> = match &p.table {
            Some(t) => vec![t.as_str()],
            None => vec!["runbooks", "knowledge", "incidents", "handoffs"],
        };

        let mut summary = serde_json::Map::new();

        for table in &tables {
            let mut processed = 0i64;
            let mut failed = 0i64;

            match *table {
                "runbooks" => {
                    if let Ok(rows) = crate::repo::embedding_repo::get_runbooks_without_embeddings(
                        &self.pool, batch_size,
                    )
                    .await
                    {
                        let texts: Vec<String> = rows
                            .iter()
                            .map(crate::embeddings::prepare_runbook_text)
                            .collect();
                        match client.embed_texts(&texts).await {
                            Ok(embeddings) => {
                                for (row, emb) in rows.iter().zip(embeddings.iter()) {
                                    if crate::repo::embedding_repo::store_runbook_embedding(
                                        &self.pool, row.id, emb,
                                    )
                                    .await
                                    .is_ok()
                                    {
                                        processed += 1;
                                    } else {
                                        failed += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                summary.insert(
                                    format!("{table}_error"),
                                    serde_json::Value::String(e.to_string()),
                                );
                            }
                        }
                    }
                }
                "knowledge" => {
                    if let Ok(rows) = crate::repo::embedding_repo::get_knowledge_without_embeddings(
                        &self.pool, batch_size,
                    )
                    .await
                    {
                        let texts: Vec<String> = rows
                            .iter()
                            .map(crate::embeddings::prepare_knowledge_text)
                            .collect();
                        match client.embed_texts(&texts).await {
                            Ok(embeddings) => {
                                for (row, emb) in rows.iter().zip(embeddings.iter()) {
                                    if crate::repo::embedding_repo::store_knowledge_embedding(
                                        &self.pool, row.id, emb,
                                    )
                                    .await
                                    .is_ok()
                                    {
                                        processed += 1;
                                    } else {
                                        failed += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                summary.insert(
                                    format!("{table}_error"),
                                    serde_json::Value::String(e.to_string()),
                                );
                            }
                        }
                    }
                }
                "incidents" => {
                    if let Ok(rows) = crate::repo::embedding_repo::get_incidents_without_embeddings(
                        &self.pool, batch_size,
                    )
                    .await
                    {
                        let texts: Vec<String> = rows
                            .iter()
                            .map(crate::embeddings::prepare_incident_text)
                            .collect();
                        match client.embed_texts(&texts).await {
                            Ok(embeddings) => {
                                for (row, emb) in rows.iter().zip(embeddings.iter()) {
                                    if crate::repo::embedding_repo::store_incident_embedding(
                                        &self.pool, row.id, emb,
                                    )
                                    .await
                                    .is_ok()
                                    {
                                        processed += 1;
                                    } else {
                                        failed += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                summary.insert(
                                    format!("{table}_error"),
                                    serde_json::Value::String(e.to_string()),
                                );
                            }
                        }
                    }
                }
                "handoffs" => {
                    if let Ok(rows) = crate::repo::embedding_repo::get_handoffs_without_embeddings(
                        &self.pool, batch_size,
                    )
                    .await
                    {
                        let texts: Vec<String> = rows
                            .iter()
                            .map(crate::embeddings::prepare_handoff_text)
                            .collect();
                        match client.embed_texts(&texts).await {
                            Ok(embeddings) => {
                                for (row, emb) in rows.iter().zip(embeddings.iter()) {
                                    if crate::repo::embedding_repo::store_handoff_embedding(
                                        &self.pool, row.id, emb,
                                    )
                                    .await
                                    .is_ok()
                                    {
                                        processed += 1;
                                    } else {
                                        failed += 1;
                                    }
                                }
                            }
                            Err(e) => {
                                summary.insert(
                                    format!("{table}_error"),
                                    serde_json::Value::String(e.to_string()),
                                );
                            }
                        }
                    }
                }
                _ => {
                    summary.insert(
                        format!("{table}_error"),
                        serde_json::Value::String("Unknown table".to_string()),
                    );
                    continue;
                }
            }

            summary.insert(
                format!("{table}_processed"),
                serde_json::Value::Number(processed.into()),
            );
            summary.insert(
                format!("{table}_failed"),
                serde_json::Value::Number(failed.into()),
            );
        }

        // Get remaining counts
        if let Ok(counts) = crate::repo::embedding_repo::count_missing_embeddings(&self.pool).await
        {
            summary.insert(
                "remaining_runbooks".to_string(),
                serde_json::Value::Number(counts.runbooks.into()),
            );
            summary.insert(
                "remaining_knowledge".to_string(),
                serde_json::Value::Number(counts.knowledge.into()),
            );
            summary.insert(
                "remaining_incidents".to_string(),
                serde_json::Value::Number(counts.incidents.into()),
            );
            summary.insert(
                "remaining_handoffs".to_string(),
                serde_json::Value::Number(counts.handoffs.into()),
            );
        }

        Ok(json_result(&serde_json::Value::Object(summary)))
    }

    // ===== MONITORING TOOLS =====

    #[tool(
        name = "list_monitors",
        description = "List all Uptime Kuma monitors with live status. Fetches real-time data from the /metrics endpoint. \
        Optionally filter by status: up, down, pending, maintenance. Shows linked server/service mappings."
    )]
    async fn list_monitors(
        &self,
        params: Parameters<monitoring::ListMonitorsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let kuma_config = match &self.kuma_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Uptime Kuma not configured (set UPTIME_KUMA_URL)",
                ))
            }
        };

        let summary = match crate::metrics::fetch_metrics(kuma_config).await {
            Ok(s) => s,
            Err(e) => return Ok(error_result(&format!("Failed to fetch metrics: {e}"))),
        };

        // Get all monitor mappings from DB
        let mappings = crate::repo::monitor_repo::list_monitors(&self.pool)
            .await
            .unwrap_or_default();

        let mut results: Vec<serde_json::Value> = Vec::new();
        for monitor in &summary.monitors {
            // Apply status filter
            if let Some(ref status_filter) = p.status {
                if monitor.status_text != *status_filter {
                    continue;
                }
            }

            let mapping = mappings.iter().find(|m| m.monitor_name == monitor.name);
            let mut val = serde_json::to_value(monitor).unwrap_or_default();
            if let Some(mapping) = mapping {
                val["linked_server_id"] =
                    serde_json::to_value(mapping.server_id).unwrap_or_default();
                val["linked_service_id"] =
                    serde_json::to_value(mapping.service_id).unwrap_or_default();
                val["mapping_notes"] = serde_json::to_value(&mapping.notes).unwrap_or_default();
            }
            results.push(val);
        }

        let output = serde_json::json!({
            "total": summary.total,
            "up": summary.up,
            "down": summary.down,
            "pending": summary.pending,
            "maintenance": summary.maintenance,
            "filtered_count": results.len(),
            "monitors": results,
        });
        Ok(json_result(&output))
    }

    #[tool(
        name = "get_monitor_status",
        description = "Get detailed live status for a specific Uptime Kuma monitor by name. \
        Shows status, response time, SSL cert info, and any linked server/service."
    )]
    async fn get_monitor_status(
        &self,
        params: Parameters<monitoring::GetMonitorStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let kuma_config = match &self.kuma_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Uptime Kuma not configured (set UPTIME_KUMA_URL)",
                ))
            }
        };

        let summary = match crate::metrics::fetch_metrics(kuma_config).await {
            Ok(s) => s,
            Err(e) => return Ok(error_result(&format!("Failed to fetch metrics: {e}"))),
        };

        let monitor = match summary.monitors.iter().find(|m| m.name == p.monitor_name) {
            Some(m) => m,
            None => return Ok(not_found("Monitor", &p.monitor_name)),
        };

        let mapping = crate::repo::monitor_repo::get_monitor_by_name(&self.pool, &p.monitor_name)
            .await
            .ok()
            .flatten();

        let mut result = serde_json::to_value(monitor).unwrap_or_default();

        // Enrich with linked entities
        if let Some(ref mapping) = mapping {
            if let Some(server_id) = mapping.server_id {
                if let Ok(Some(server)) =
                    crate::repo::server_repo::get_server(&self.pool, server_id).await
                {
                    result["linked_server"] = serde_json::to_value(&server).unwrap_or_default();
                }
            }
            if let Some(service_id) = mapping.service_id {
                if let Ok(Some(service)) =
                    crate::repo::service_repo::get_service(&self.pool, service_id).await
                {
                    result["linked_service"] = serde_json::to_value(&service).unwrap_or_default();
                }
            }
            result["mapping_notes"] = serde_json::to_value(&mapping.notes).unwrap_or_default();
        }

        Ok(json_result(&result))
    }

    #[tool(
        name = "get_monitoring_summary",
        description = "Get a high-level monitoring overview: total monitors, how many are up/down/pending/maintenance, \
        and a list of any monitors currently DOWN. Quick health check for all infrastructure."
    )]
    async fn get_monitoring_summary(
        &self,
        _params: Parameters<monitoring::GetMonitoringSummaryParams>,
    ) -> Result<CallToolResult, McpError> {
        let kuma_config = match &self.kuma_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Uptime Kuma not configured (set UPTIME_KUMA_URL)",
                ))
            }
        };

        let summary = match crate::metrics::fetch_metrics(kuma_config).await {
            Ok(s) => s,
            Err(e) => return Ok(error_result(&format!("Failed to fetch metrics: {e}"))),
        };

        // Highlight anything that's down
        let down_monitors: Vec<&crate::metrics::MonitorStatus> =
            summary.monitors.iter().filter(|m| m.status == 0).collect();

        let result = serde_json::json!({
            "status": if summary.down == 0 { "ALL_CLEAR" } else { "DEGRADED" },
            "total": summary.total,
            "up": summary.up,
            "down": summary.down,
            "pending": summary.pending,
            "maintenance": summary.maintenance,
            "down_monitors": down_monitors,
        });
        Ok(json_result(&result))
    }

    #[tool(
        name = "link_monitor",
        description = "Link an Uptime Kuma monitor to an ops-brain server and/or service. \
        This mapping enriches get_situational_awareness with live monitoring data. \
        The monitor_name must match exactly as shown in list_monitors."
    )]
    async fn link_monitor(
        &self,
        params: Parameters<monitoring::LinkMonitorParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        // Resolve server slug to ID
        let server_id = match &p.server_slug {
            Some(slug) => {
                match crate::repo::server_repo::get_server_by_slug(&self.pool, slug).await {
                    Ok(Some(s)) => Some(s.id),
                    Ok(None) => return Ok(not_found("Server", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        // Resolve service slug to ID
        let service_id = match &p.service_slug {
            Some(slug) => {
                match crate::repo::service_repo::get_service_by_slug(&self.pool, slug).await {
                    Ok(Some(s)) => Some(s.id),
                    Ok(None) => return Ok(not_found("Service", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        if server_id.is_none() && service_id.is_none() && p.notes.is_none() {
            return Ok(error_result(
                "Provide at least one of: server_slug, service_slug, or notes",
            ));
        }

        match crate::repo::monitor_repo::upsert_monitor(
            &self.pool,
            &p.monitor_name,
            server_id,
            service_id,
            p.notes.as_deref(),
        )
        .await
        {
            Ok(monitor) => Ok(json_result(&monitor)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "unlink_monitor",
        description = "Remove the mapping between an Uptime Kuma monitor and ops-brain entities. \
        The monitor will still appear in list_monitors but won't be linked to any server/service."
    )]
    async fn unlink_monitor(
        &self,
        params: Parameters<monitoring::UnlinkMonitorParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::monitor_repo::delete_monitor(&self.pool, &p.monitor_name).await {
            Ok(true) => Ok(CallToolResult::success(vec![Content::text(format!(
                "Monitor mapping removed: {}",
                p.monitor_name
            ))])),
            Ok(false) => Ok(not_found("Monitor mapping", &p.monitor_name)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "list_watchdog_incidents",
        description = "List incidents auto-created by the proactive monitoring watchdog. \
        These are incidents created when Uptime Kuma monitors transition to DOWN, \
        and auto-resolved when they recover. Useful for reviewing outage history and patterns."
    )]
    async fn list_watchdog_incidents(
        &self,
        params: Parameters<monitoring::ListWatchdogIncidentsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let limit = p.limit.unwrap_or(20);
        let prefix_pattern = format!("{}%", crate::watchdog::INCIDENT_PREFIX);

        let query = match &p.status {
            Some(status) => {
                sqlx::query_as::<_, Incident>(
                    "SELECT * FROM incidents WHERE title LIKE $1 AND status = $2 ORDER BY reported_at DESC LIMIT $3",
                )
                .bind(&prefix_pattern)
                .bind(status)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
            None => {
                sqlx::query_as::<_, Incident>(
                    "SELECT * FROM incidents WHERE title LIKE $1 ORDER BY reported_at DESC LIMIT $2",
                )
                .bind(&prefix_pattern)
                .bind(limit)
                .fetch_all(&self.pool)
                .await
            }
        };

        match query {
            Ok(incidents) => {
                let result = serde_json::json!({
                    "count": incidents.len(),
                    "incidents": incidents,
                });
                Ok(json_result(&result))
            }
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    // ===== ZAMMAD TICKET TOOLS =====

    #[tool(
        name = "list_tickets",
        description = "List Zammad tickets for a client, filtered by state and priority. Requires client_slug to resolve the Zammad organization."
    )]
    async fn list_tickets(
        &self,
        params: Parameters<zammad::ListTicketsParams>,
    ) -> Result<CallToolResult, McpError> {
        let zammad = match &self.zammad_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)",
                ))
            }
        };
        let p = params.0;

        let client =
            match crate::repo::client_repo::get_client_by_slug(&self.pool, &p.client_slug).await {
                Ok(Some(c)) => c,
                Ok(None) => return Ok(not_found("Client", &p.client_slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        let org_id = match client.zammad_org_id {
            Some(id) => id,
            None => return Ok(error_result(&format!(
                "Client '{}' has no Zammad org ID configured. Use upsert_client to set zammad_org_id.",
                p.client_slug
            ))),
        };

        let mut query_parts = vec![format!("organization.id:{org_id}")];
        if let Some(ref state) = p.state {
            query_parts.push(format!("state.name:{state}"));
        }
        if let Some(ref priority) = p.priority {
            query_parts.push(format!("priority.name:\"{priority}\""));
        }
        let query = query_parts.join(" AND ");
        let limit = p.limit.unwrap_or(20);

        match crate::zammad::search_tickets(zammad, &query, limit).await {
            Ok(tickets) => {
                let result = serde_json::json!({
                    "count": tickets.len(),
                    "client": p.client_slug,
                    "tickets": tickets,
                });
                Ok(json_result(&result))
            }
            Err(e) => Ok(error_result(&e)),
        }
    }

    #[tool(
        name = "get_ticket",
        description = "Get a Zammad ticket by ID with full article history (messages, notes, time accounting)."
    )]
    async fn get_ticket(
        &self,
        params: Parameters<zammad::GetTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        let zammad = match &self.zammad_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)",
                ))
            }
        };
        let p = params.0;

        let ticket = match crate::zammad::get_ticket(zammad, p.ticket_id).await {
            Ok(t) => t,
            Err(e) => return Ok(error_result(&e)),
        };

        let articles = match crate::zammad::get_ticket_articles(zammad, p.ticket_id).await {
            Ok(a) => a,
            Err(e) => return Ok(error_result(&e)),
        };

        let link =
            crate::repo::ticket_link_repo::get_link_by_ticket_id(&self.pool, p.ticket_id as i32)
                .await
                .ok()
                .flatten();

        let result = serde_json::json!({
            "ticket": ticket,
            "articles": articles,
            "ops_brain_link": link,
        });
        Ok(json_result(&result))
    }

    #[tool(
        name = "create_ticket",
        description = "Create a new ticket in Zammad. Resolves client_slug to Zammad group/org/customer. Optionally links to an ops-brain incident."
    )]
    async fn create_ticket(
        &self,
        params: Parameters<zammad::CreateTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        let zammad = match &self.zammad_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)",
                ))
            }
        };
        let p = params.0;

        let client =
            match crate::repo::client_repo::get_client_by_slug(&self.pool, &p.client_slug).await {
                Ok(Some(c)) => c,
                Ok(None) => return Ok(not_found("Client", &p.client_slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        let (group_id, customer_id, org_id) = match (client.zammad_group_id, client.zammad_customer_id, client.zammad_org_id) {
            (Some(g), Some(c), org) => (g as i64, c as i64, org.map(|o| o as i64)),
            _ => return Ok(error_result(&format!(
                "Client '{}' missing Zammad IDs. Set zammad_group_id and zammad_customer_id via upsert_client.",
                p.client_slug
            ))),
        };

        let state_id = match &p.state {
            Some(s) => match crate::zammad::state_name_to_id(s) {
                Some(id) => Some(id),
                None => {
                    return Ok(error_result(&format!(
                        "Unknown state: '{s}'. Use: new, open, pending_reminder, closed"
                    )))
                }
            },
            None => None,
        };

        let priority_id = match &p.priority {
            Some(pr) => match crate::zammad::priority_name_to_id(pr) {
                Some(id) => Some(id),
                None => {
                    return Ok(error_result(&format!(
                        "Unknown priority: '{pr}'. Use: low, normal, high"
                    )))
                }
            },
            None => None,
        };

        let payload = crate::zammad::CreateTicketPayload {
            title: p.title,
            group_id,
            customer_id,
            organization_id: org_id,
            state_id,
            priority_id,
            owner_id: Some(3), // Eduardo
            tags: p.tags,
            article: crate::zammad::CreateArticleInline {
                body: p.body,
                content_type: Some("text/plain".to_string()),
                article_type: Some("note".to_string()),
                internal: Some(false),
                time_unit: p.time_unit,
                time_accounting_type_id: p.time_accounting_type_id,
            },
        };

        let ticket = match crate::zammad::create_ticket(zammad, &payload).await {
            Ok(t) => t,
            Err(e) => return Ok(error_result(&e)),
        };

        // Auto-link to incident if provided
        if let Some(ref incident_id_str) = p.incident_id {
            if let Ok(incident_id) = uuid::Uuid::parse_str(incident_id_str) {
                let _ = crate::repo::ticket_link_repo::create_link(
                    &self.pool,
                    ticket.id as i32,
                    Some(incident_id),
                    None,
                    None,
                    None,
                )
                .await;
            }
        }

        Ok(json_result(&ticket))
    }

    #[tool(
        name = "update_ticket",
        description = "Update a Zammad ticket's state, priority, or title."
    )]
    async fn update_ticket(
        &self,
        params: Parameters<zammad::UpdateTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        let zammad = match &self.zammad_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)",
                ))
            }
        };
        let p = params.0;

        let state_id = match &p.state {
            Some(s) => match crate::zammad::state_name_to_id(s) {
                Some(id) => Some(id),
                None => {
                    return Ok(error_result(&format!(
                        "Unknown state: '{s}'. Use: new, open, pending_reminder, closed"
                    )))
                }
            },
            None => None,
        };

        let priority_id = match &p.priority {
            Some(pr) => match crate::zammad::priority_name_to_id(pr) {
                Some(id) => Some(id),
                None => {
                    return Ok(error_result(&format!(
                        "Unknown priority: '{pr}'. Use: low, normal, high"
                    )))
                }
            },
            None => None,
        };

        let payload = crate::zammad::UpdateTicketPayload {
            title: p.title,
            state_id,
            priority_id,
            owner_id: None,
        };

        match crate::zammad::update_ticket(zammad, p.ticket_id, &payload).await {
            Ok(ticket) => Ok(json_result(&ticket)),
            Err(e) => Ok(error_result(&e)),
        }
    }

    #[tool(
        name = "add_ticket_note",
        description = "Add an internal note (or public reply) to a Zammad ticket. Supports time accounting."
    )]
    async fn add_ticket_note(
        &self,
        params: Parameters<zammad::AddTicketNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        let zammad = match &self.zammad_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)",
                ))
            }
        };
        let p = params.0;

        let payload = crate::zammad::CreateArticlePayload {
            ticket_id: p.ticket_id,
            body: p.body,
            content_type: Some("text/plain".to_string()),
            article_type: Some("note".to_string()),
            internal: Some(p.internal.unwrap_or(true)),
            time_unit: p.time_unit,
            time_accounting_type_id: p.time_accounting_type_id,
        };

        match crate::zammad::add_ticket_article(zammad, &payload).await {
            Ok(article) => Ok(json_result(&article)),
            Err(e) => Ok(error_result(&e)),
        }
    }

    #[tool(
        name = "search_tickets",
        description = "Search Zammad tickets using full-text search (Elasticsearch syntax). Examples: 'soporte-usuario', 'backup failed', 'title:servidor'."
    )]
    async fn search_tickets(
        &self,
        params: Parameters<zammad::SearchTicketsParams>,
    ) -> Result<CallToolResult, McpError> {
        let zammad = match &self.zammad_config {
            Some(c) => c,
            None => {
                return Ok(error_result(
                    "Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)",
                ))
            }
        };
        let p = params.0;
        let limit = p.limit.unwrap_or(20);

        match crate::zammad::search_tickets(zammad, &p.query, limit).await {
            Ok(tickets) => {
                let result = serde_json::json!({
                    "count": tickets.len(),
                    "query": p.query,
                    "tickets": tickets,
                });
                Ok(json_result(&result))
            }
            Err(e) => Ok(error_result(&e)),
        }
    }

    #[tool(
        name = "link_ticket",
        description = "Link a Zammad ticket to ops-brain entities (incident, server, service). At least one entity must be provided."
    )]
    async fn link_ticket(
        &self,
        params: Parameters<zammad::LinkTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        if p.incident_id.is_none() && p.server_slug.is_none() && p.service_slug.is_none() {
            return Ok(error_result(
                "At least one of incident_id, server_slug, or service_slug must be provided",
            ));
        }

        let incident_id = match &p.incident_id {
            Some(id_str) => match uuid::Uuid::parse_str(id_str) {
                Ok(id) => match crate::repo::incident_repo::get_incident(&self.pool, id).await {
                    Ok(Some(_)) => Some(id),
                    Ok(None) => return Ok(not_found("Incident", id_str)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                },
                Err(_) => return Ok(error_result(&format!("Invalid incident UUID: {}", id_str))),
            },
            None => None,
        };

        let server_id = match &p.server_slug {
            Some(slug) => {
                match crate::repo::server_repo::get_server_by_slug(&self.pool, slug).await {
                    Ok(Some(s)) => Some(s.id),
                    Ok(None) => return Ok(not_found("Server", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        let service_id = match &p.service_slug {
            Some(slug) => {
                match crate::repo::service_repo::get_service_by_slug(&self.pool, slug).await {
                    Ok(Some(s)) => Some(s.id),
                    Ok(None) => return Ok(not_found("Service", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        match crate::repo::ticket_link_repo::create_link(
            &self.pool,
            p.zammad_ticket_id as i32,
            incident_id,
            server_id,
            service_id,
            p.notes.as_deref(),
        )
        .await
        {
            Ok(link) => Ok(json_result(&link)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "unlink_ticket",
        description = "Remove the link between a Zammad ticket and ops-brain entities."
    )]
    async fn unlink_ticket(
        &self,
        params: Parameters<zammad::UnlinkTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::ticket_link_repo::delete_link(&self.pool, p.zammad_ticket_id as i32)
            .await
        {
            Ok(true) => {
                let result = serde_json::json!({
                    "status": "unlinked",
                    "zammad_ticket_id": p.zammad_ticket_id,
                });
                Ok(json_result(&result))
            }
            Ok(false) => Ok(error_result(&format!(
                "No link found for Zammad ticket {}",
                p.zammad_ticket_id
            ))),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    // ===== BRIEFING TOOLS =====

    #[tool(
        name = "generate_briefing",
        description = "Generate an operational briefing (daily or weekly). Aggregates monitoring health, \
        open incidents, watchdog alerts, pending handoffs, and Zammad ticket activity into a \
        structured summary. Optionally scoped to a specific client. The briefing is stored for \
        historical reference and returned as both structured data and markdown."
    )]
    async fn generate_briefing(
        &self,
        params: Parameters<briefings::GenerateBriefingParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;

        if let Err(msg) = crate::validation::validate_required(
            &p.briefing_type,
            "briefing_type",
            crate::validation::BRIEFING_TYPES,
        ) {
            return Ok(error_result(&msg));
        }

        let client = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        match crate::api::generate_briefing_inner(
            &self.pool,
            &self.kuma_config,
            &self.zammad_config,
            &p.briefing_type.to_lowercase(),
            client.as_ref(),
        )
        .await
        {
            Ok(output) => Ok(json_result(&output)),
            Err(e) => Ok(error_result(&e)),
        }
    }

    #[tool(
        name = "list_briefings",
        description = "List previously generated briefings. Filter by type (daily/weekly) and/or client slug. \
        Returns metadata and content of each briefing, ordered by most recent first."
    )]
    async fn list_briefings(
        &self,
        params: Parameters<briefings::ListBriefingsParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let limit = p.limit.unwrap_or(10);

        if let Err(msg) = crate::validation::validate_option(
            p.briefing_type.as_deref(),
            "briefing_type",
            crate::validation::BRIEFING_TYPES,
        ) {
            return Ok(error_result(&msg));
        }

        let client_id = match &p.client_slug {
            Some(slug) => {
                match crate::repo::client_repo::get_client_by_slug(&self.pool, slug).await {
                    Ok(Some(c)) => Some(c.id),
                    Ok(None) => return Ok(not_found("Client", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
            None => None,
        };

        match crate::repo::briefing_repo::list_briefings(
            &self.pool,
            p.briefing_type.as_deref(),
            client_id,
            limit,
        )
        .await
        {
            Ok(briefings) => {
                let result = serde_json::json!({
                    "count": briefings.len(),
                    "briefings": briefings,
                });
                Ok(json_result(&result))
            }
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "get_briefing",
        description = "Retrieve a specific briefing by ID."
    )]
    async fn get_briefing(
        &self,
        params: Parameters<briefings::GetBriefingParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        let id = match uuid::Uuid::parse_str(&p.id) {
            Ok(id) => id,
            Err(_) => return Ok(error_result(&format!("Invalid UUID: {}", p.id))),
        };

        match crate::repo::briefing_repo::get_briefing(&self.pool, id).await {
            Ok(Some(briefing)) => Ok(json_result(&briefing)),
            Ok(None) => Ok(not_found("Briefing", &p.id)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    // ── Delete tools (inventory cleanup) ──────────────────────────────

    #[tool(
        name = "delete_server",
        description = "Delete a server by slug. Without confirm=true, returns a preview of linked entities that would be affected. \
        Junction table links (incidents, runbooks, services, monitors, tickets) are cascade-deleted or set null. \
        Child VMs referencing this server as hypervisor will have hypervisor_id set to null."
    )]
    async fn delete_server(
        &self,
        params: Parameters<inventory::DeleteServerParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let server =
            match crate::repo::server_repo::get_server_by_slug(&self.pool, &params.slug).await {
                Ok(Some(s)) => s,
                Ok(None) => return Ok(not_found("Server", &params.slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        let refs =
            match crate::repo::server_repo::count_server_references(&self.pool, server.id).await {
                Ok(r) => r,
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
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
                    "Junction table links will be CASCADE-deleted or SET NULL.".into();
            } else {
                preview["linked_entities"] = serde_json::json!("none");
            }
            return Ok(json_result(&preview));
        }

        match crate::repo::server_repo::delete_server(&self.pool, server.id).await {
            Ok(true) => Ok(json_result(&serde_json::json!({
                "deleted": true,
                "server": server.hostname,
                "slug": server.slug,
                "cascade_summary": refs.iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            }))),
            Ok(false) => Ok(not_found("Server", &params.slug)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "delete_service",
        description = "Delete a service by slug. Without confirm=true, returns a preview of linked entities that would be affected. \
        Junction table links (servers, incidents, runbooks, monitors, tickets) are cascade-deleted or set null."
    )]
    async fn delete_service(
        &self,
        params: Parameters<inventory::DeleteServiceParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let service =
            match crate::repo::service_repo::get_service_by_slug(&self.pool, &params.slug).await {
                Ok(Some(s)) => s,
                Ok(None) => return Ok(not_found("Service", &params.slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        let refs = match crate::repo::service_repo::count_service_references(&self.pool, service.id)
            .await
        {
            Ok(r) => r,
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
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
                    "Junction table links will be CASCADE-deleted or SET NULL.".into();
            } else {
                preview["linked_entities"] = serde_json::json!("none");
            }
            return Ok(json_result(&preview));
        }

        match crate::repo::service_repo::delete_service(&self.pool, service.id).await {
            Ok(true) => Ok(json_result(&serde_json::json!({
                "deleted": true,
                "service": service.name,
                "slug": service.slug,
                "cascade_summary": refs.iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            }))),
            Ok(false) => Ok(not_found("Service", &params.slug)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "delete_vendor",
        description = "Delete a vendor by name (case-insensitive). Without confirm=true, returns a preview of linked entities. \
        Client links and incident links are cascade-deleted."
    )]
    async fn delete_vendor(
        &self,
        params: Parameters<inventory::DeleteVendorParams>,
    ) -> Result<CallToolResult, McpError> {
        let params = params.0;
        let vendor =
            match crate::repo::vendor_repo::get_vendor_by_name(&self.pool, &params.name).await {
                Ok(Some(v)) => v,
                Ok(None) => return Ok(not_found("Vendor", &params.name)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        let refs =
            match crate::repo::vendor_repo::count_vendor_references(&self.pool, vendor.id).await {
                Ok(r) => r,
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
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
                    "Client links and incident links will be CASCADE-deleted.".into();
            } else {
                preview["linked_entities"] = serde_json::json!("none");
            }
            return Ok(json_result(&preview));
        }

        match crate::repo::vendor_repo::delete_vendor(&self.pool, vendor.id).await {
            Ok(true) => Ok(json_result(&serde_json::json!({
                "deleted": true,
                "vendor": vendor.name,
                "id": vendor.id.to_string(),
                "cascade_summary": refs.iter()
                    .map(|(k, v)| format!("{k}: {v}"))
                    .collect::<Vec<_>>()
                    .join(", "),
            }))),
            Ok(false) => Ok(not_found("Vendor", &params.name)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }
}

#[tool_handler]
impl ServerHandler for OpsBrain {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ops-brain", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "Operational intelligence server for IT infrastructure management. \
                 Use get_situational_awareness for comprehensive context about any \
                 server, service, or client. Use search_inventory for full-text search. \
                 Use semantic_search for AI-powered conceptual search across runbooks, \
                 knowledge, incidents, and handoffs (finds related content even without \
                 exact keyword matches). Use get_monitoring_summary for live infrastructure \
                 health from Uptime Kuma. Use list_tickets, search_tickets, and get_ticket \
                 for Zammad ticketing integration — create_ticket and add_ticket_note for \
                 ticket management with time accounting. Use generate_briefing for \
                 daily/weekly operational summaries aggregating monitoring, incidents, \
                 handoffs, and tickets.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_item(id: Uuid, client_id: Option<Uuid>, cross_client_safe: bool) -> serde_json::Value {
        let mut obj = serde_json::json!({
            "id": id.to_string(),
            "title": "Test Item",
            "cross_client_safe": cross_client_safe,
        });
        if let Some(cid) = client_id {
            obj["client_id"] = serde_json::Value::String(cid.to_string());
        }
        obj
    }

    fn make_lookup() -> (Uuid, Uuid, HashMap<Uuid, (String, String)>) {
        let hsr_id = Uuid::now_v7();
        let cpa_id = Uuid::now_v7();
        let mut lookup = HashMap::new();
        lookup.insert(hsr_id, ("hsr".to_string(), "Hospice".to_string()));
        lookup.insert(cpa_id, ("cpa".to_string(), "CPA Firm".to_string()));
        (hsr_id, cpa_id, lookup)
    }

    // ===== filter_cross_client tests =====

    #[test]
    fn filter_no_requesting_client_allows_all() {
        let (hsr_id, _, lookup) = make_lookup();
        let items = vec![
            make_item(Uuid::now_v7(), Some(hsr_id), false),
            make_item(Uuid::now_v7(), None, false),
        ];

        let result = filter_cross_client(items, "runbook", None, false, &lookup);

        assert_eq!(result.allowed.len(), 2);
        assert!(result.withheld_notices.is_empty());
        assert!(result.audit_entries.is_empty());
    }

    #[test]
    fn filter_global_content_always_allowed() {
        let (hsr_id, _, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, None, false)];

        let result = filter_cross_client(items, "runbook", Some(hsr_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert!(result.audit_entries.is_empty());
        // Provenance should show "Global"
        assert_eq!(result.allowed[0]["_client_name"], "Global");
        assert!(result.allowed[0]["_client_slug"].is_null());
    }

    #[test]
    fn filter_same_client_allowed() {
        let (hsr_id, _, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(hsr_id), false)];

        let result = filter_cross_client(items, "runbook", Some(hsr_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert!(result.audit_entries.is_empty());
        // Provenance should show the client
        assert_eq!(result.allowed[0]["_client_slug"], "hsr");
        assert_eq!(result.allowed[0]["_client_name"], "Hospice");
    }

    #[test]
    fn filter_cross_client_safe_allowed() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        // HSR item marked as cross_client_safe, requesting from CPA
        let items = vec![make_item(item_id, Some(hsr_id), true)];

        let result = filter_cross_client(items, "runbook", Some(cpa_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        // Audit entry should record "released_safe"
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].0, item_id);
        assert_eq!(result.audit_entries[0].1, Some(hsr_id));
        assert_eq!(result.audit_entries[0].2, "released_safe");
    }

    #[test]
    fn filter_cross_client_acknowledged_released() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        // HSR item NOT safe, but acknowledge=true
        let items = vec![make_item(item_id, Some(hsr_id), false)];

        let result = filter_cross_client(items, "runbook", Some(cpa_id), true, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        // Audit entry should record "released"
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "released");
    }

    #[test]
    fn filter_cross_client_withheld() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        // HSR item, NOT safe, NOT acknowledged, requesting from CPA
        let items = vec![make_item(item_id, Some(hsr_id), false)];

        let result = filter_cross_client(items, "runbook", Some(cpa_id), false, &lookup);

        assert!(result.allowed.is_empty());
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["count"], 1);
        assert_eq!(result.withheld_notices[0]["owning_client_slug"], "hsr");
        assert_eq!(result.withheld_notices[0]["entity_type"], "runbook");
        // Audit entry should record "withheld"
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "withheld");
    }

    #[test]
    fn filter_multiple_withheld_grouped_by_client() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let items = vec![
            make_item(Uuid::now_v7(), Some(hsr_id), false),
            make_item(Uuid::now_v7(), Some(hsr_id), false),
        ];

        let result = filter_cross_client(items, "knowledge", Some(cpa_id), false, &lookup);

        assert!(result.allowed.is_empty());
        // Should be grouped into one notice (both from HSR)
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["count"], 2);
        assert_eq!(result.audit_entries.len(), 2);
    }

    #[test]
    fn filter_mixed_items() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let items = vec![
            make_item(Uuid::now_v7(), None, false), // global → allowed
            make_item(Uuid::now_v7(), Some(cpa_id), false), // same client → allowed
            make_item(Uuid::now_v7(), Some(hsr_id), true), // diff client, safe → allowed
            make_item(Uuid::now_v7(), Some(hsr_id), false), // diff client, not safe → withheld
        ];

        let result = filter_cross_client(items, "runbook", Some(cpa_id), false, &lookup);

        assert_eq!(result.allowed.len(), 3);
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["count"], 1);
        // 1 released_safe + 1 withheld
        assert_eq!(result.audit_entries.len(), 2);
    }

    // ===== inject_provenance tests =====

    #[test]
    fn provenance_with_client() {
        let (hsr_id, _, lookup) = make_lookup();
        let mut item = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
            "client_id": hsr_id.to_string(),
        });

        inject_provenance(&mut item, &lookup);

        assert_eq!(item["_client_slug"], "hsr");
        assert_eq!(item["_client_name"], "Hospice");
    }

    #[test]
    fn provenance_without_client() {
        let (_, _, lookup) = make_lookup();
        let mut item = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
        });

        inject_provenance(&mut item, &lookup);

        assert!(item["_client_slug"].is_null());
        assert_eq!(item["_client_name"], "Global");
    }

    #[test]
    fn provenance_unknown_client() {
        let lookup = HashMap::new();
        let unknown_id = Uuid::now_v7();
        let mut item = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
            "client_id": unknown_id.to_string(),
        });

        inject_provenance(&mut item, &lookup);

        // Unknown client_id — no provenance injected (no crash)
        assert!(item.get("_client_slug").is_none());
    }
}
