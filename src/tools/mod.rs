mod context;
mod inventory;
mod knowledge;
mod runbooks;

use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use serde::Serialize;
use sqlx::PgPool;

use crate::models::handoff::Handoff;
use crate::models::incident::Incident;

#[derive(Clone)]
pub struct OpsBrain {
    pool: PgPool,
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

#[tool_router]
impl OpsBrain {
    pub fn new(pool: PgPool) -> Self {
        Self {
            pool,
            tool_router: Self::tool_router(),
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
        let server = match crate::repo::server_repo::get_server_by_slug(&self.pool, &p.slug).await
        {
            Ok(Some(s)) => s,
            Ok(None) => return Ok(not_found("Server", &p.slug)),
            Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
        };
        let services =
            crate::repo::service_repo::get_services_for_server(&self.pool, server.id)
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
            Some(slug) => {
                match crate::repo::site_repo::get_site_by_slug(&self.pool, slug).await {
                    Ok(Some(s)) => Some(s.id),
                    Ok(None) => return Ok(not_found("Site", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
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
        let servers = crate::repo::server_repo::list_servers(
            &self.pool,
            None,
            Some(site.id),
            None,
            None,
        )
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
            Some(slug) => {
                match crate::repo::site_repo::get_site_by_slug(&self.pool, slug).await {
                    Ok(Some(s)) => Some(s.id),
                    Ok(None) => return Ok(not_found("Site", slug)),
                    Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
                }
            }
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

        let site =
            match crate::repo::site_repo::get_site_by_slug(&self.pool, &p.site_slug).await {
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
        )
        .await
        {
            Ok(runbooks) => Ok(json_result(&runbooks)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "search_runbooks",
        description = "Full-text search across runbook titles and content"
    )]
    async fn search_runbooks(
        &self,
        params: Parameters<runbooks::SearchRunbooksParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::search_repo::search_runbooks(&self.pool, &p.query).await {
            Ok(runbooks) => Ok(json_result(&runbooks)),
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
        )
        .await
        {
            Ok(runbook) => Ok(json_result(&runbook)),
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
        )
        .await
        {
            Ok(updated) => Ok(json_result(&updated)),
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
        )
        .await
        {
            Ok(entry) => Ok(json_result(&entry)),
            Err(e) => Ok(error_result(&format!("Database error: {e}"))),
        }
    }

    #[tool(
        name = "search_knowledge",
        description = "Full-text search across knowledge base entries"
    )]
    async fn search_knowledge(
        &self,
        params: Parameters<knowledge::SearchKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        let p = params.0;
        match crate::repo::knowledge_repo::search_knowledge(&self.pool, &p.query).await {
            Ok(entries) => Ok(json_result(&entries)),
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
                    if !awareness.recent_incidents.iter().any(|existing| {
                        existing.get("id") == val.get("id")
                    }) {
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
                    if !awareness.recent_incidents.iter().any(|existing| {
                        existing.get("id") == val.get("id")
                    }) {
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
                    if !awareness.knowledge.iter().any(|existing| {
                        existing.get("id") == val.get("id")
                    }) {
                        awareness.knowledge.push(val);
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

        let servers = crate::repo::server_repo::list_servers(
            &self.pool,
            Some(client.id),
            None,
            None,
            None,
        )
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

        let overview = context::ClientOverview {
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
        };

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

        let server =
            match crate::repo::server_repo::get_server_by_slug(&self.pool, &p.server_slug).await {
                Ok(Some(s)) => s,
                Ok(None) => return Ok(not_found("Server", &p.server_slug)),
                Err(e) => return Ok(error_result(&format!("Database error: {e}"))),
            };

        let services =
            crate::repo::service_repo::get_services_for_server(&self.pool, server.id)
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
        let knowledge = if let Some(cid) = client_id {
            crate::repo::knowledge_repo::list_knowledge(&self.pool, None, Some(cid))
                .await
                .unwrap_or_default()
        } else {
            Vec::new()
        };

        let result = serde_json::json!({
            "server": server,
            "services": services,
            "site": site,
            "client": client,
            "networks": networks,
            "vendors": vendors,
            "recent_incidents": all_incidents,
            "runbooks": all_runbooks,
            "knowledge": knowledge,
        });

        Ok(json_result(&result))
    }
}

#[tool_handler]
impl ServerHandler for OpsBrain {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new(
                "ops-brain",
                env!("CARGO_PKG_VERSION"),
            ))
            .with_instructions(
                "Operational intelligence server for IT infrastructure management. \
                 Use get_situational_awareness for comprehensive context about any \
                 server, service, or client. Use search_inventory for full-text search.",
            )
    }
}
