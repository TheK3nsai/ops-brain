pub mod briefings;
mod context;
mod coordination;
mod helpers;
mod incidents;
mod inventory;
mod knowledge;
mod monitoring;
mod runbooks;
mod search;
mod shared;
mod zammad;

use rmcp::{
    handler::server::{tool::ToolRouter, wrapper::Parameters},
    model::*,
    tool, tool_handler, tool_router, ErrorData as McpError, ServerHandler,
};
use sqlx::PgPool;

use crate::embeddings::EmbeddingClient;
use crate::metrics::UptimeKumaConfig;
use crate::zammad::ZammadConfig;

#[derive(Clone)]
pub struct OpsBrain {
    pub(crate) pool: PgPool,
    pub(crate) kuma_config: Option<UptimeKumaConfig>,
    pub(crate) embedding_client: Option<EmbeddingClient>,
    pub(crate) zammad_config: Option<ZammadConfig>,
    tool_router: ToolRouter<Self>,
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

    // ===== INVENTORY: READ TOOLS =====

    #[tool(
        name = "get_server",
        description = "Get detailed information about a server including its services, site, and network configuration"
    )]
    async fn get_server(
        &self,
        params: Parameters<inventory::GetServerParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_get_server(self, params.0).await)
    }

    #[tool(
        name = "list_servers",
        description = "List servers with optional filters by client, site, role, or status"
    )]
    async fn list_servers(
        &self,
        params: Parameters<inventory::ListServersParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_list_servers(self, params.0).await)
    }

    #[tool(
        name = "get_service",
        description = "Get detailed information about a service and which servers run it"
    )]
    async fn get_service(
        &self,
        params: Parameters<inventory::GetServiceParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_get_service(self, params.0).await)
    }

    #[tool(
        name = "list_services",
        description = "List all services, optionally filtered by category"
    )]
    async fn list_services(
        &self,
        params: Parameters<inventory::ListServicesParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_list_services(self, params.0).await)
    }

    #[tool(
        name = "get_site",
        description = "Get detailed information about a site including its servers and networks"
    )]
    async fn get_site(
        &self,
        params: Parameters<inventory::GetSiteParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_get_site(self, params.0).await)
    }

    #[tool(name = "get_client", description = "Get client information by slug")]
    async fn get_client(
        &self,
        params: Parameters<inventory::GetClientParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_get_client(self, params.0).await)
    }

    #[tool(
        name = "get_network",
        description = "Get network information by site slug or network ID"
    )]
    async fn get_network(
        &self,
        params: Parameters<inventory::GetNetworkParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_get_network(self, params.0).await)
    }

    #[tool(
        name = "get_vendor",
        description = "Get vendor information by name (case-insensitive) or ID. Use ID to disambiguate when multiple vendors share a name."
    )]
    async fn get_vendor(
        &self,
        params: Parameters<inventory::GetVendorParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_get_vendor(self, params.0).await)
    }

    #[tool(
        name = "list_vendors",
        description = "List all vendors, optionally filtered by category or client"
    )]
    async fn list_vendors(
        &self,
        params: Parameters<inventory::ListVendorsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_list_vendors(self, params.0).await)
    }

    #[tool(name = "list_clients", description = "List all client organizations")]
    async fn list_clients(
        &self,
        params: Parameters<inventory::ListClientsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_list_clients(self, params.0).await)
    }

    #[tool(
        name = "list_sites",
        description = "List all sites, optionally filtered by client"
    )]
    async fn list_sites(
        &self,
        params: Parameters<inventory::ListSitesParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_list_sites(self, params.0).await)
    }

    #[tool(
        name = "list_networks",
        description = "List all networks, optionally filtered by site"
    )]
    async fn list_networks(
        &self,
        params: Parameters<inventory::ListNetworksParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_list_networks(self, params.0).await)
    }

    #[tool(
        name = "search_inventory",
        description = "Full-text search across servers, services, runbooks, knowledge, incidents, handoffs, vendors, clients, sites, and networks"
    )]
    async fn search_inventory(
        &self,
        params: Parameters<inventory::SearchInventoryParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_search_inventory(self, params.0).await)
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
        Ok(inventory::handle_upsert_client(self, params.0).await)
    }

    #[tool(
        name = "upsert_site",
        description = "Create or update a site. Resolves client_slug to find the parent client."
    )]
    async fn upsert_site(
        &self,
        params: Parameters<inventory::UpsertSiteParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_upsert_site(self, params.0).await)
    }

    #[tool(
        name = "upsert_server",
        description = "Create or update a server. Resolves site_slug and optional hypervisor_slug to IDs."
    )]
    async fn upsert_server(
        &self,
        params: Parameters<inventory::UpsertServerParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_upsert_server(self, params.0).await)
    }

    #[tool(
        name = "upsert_service",
        description = "Create or update a service definition. Updates existing if slug matches."
    )]
    async fn upsert_service(
        &self,
        params: Parameters<inventory::UpsertServiceParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_upsert_service(self, params.0).await)
    }

    #[tool(
        name = "upsert_vendor",
        description = "Create or update a vendor contact record. Provide id to update a specific vendor by UUID."
    )]
    async fn upsert_vendor(
        &self,
        params: Parameters<inventory::UpsertVendorParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_upsert_vendor(self, params.0).await)
    }

    #[tool(
        name = "link_server_service",
        description = "Link a server to a service it runs, with optional port and config notes"
    )]
    async fn link_server_service(
        &self,
        params: Parameters<inventory::LinkServerServiceParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_link_server_service(self, params.0).await)
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
        Ok(runbooks::handle_get_runbook(self, params.0).await)
    }

    #[tool(
        name = "list_runbooks",
        description = "List runbooks with optional filters by category, service, server, or tag"
    )]
    async fn list_runbooks(
        &self,
        params: Parameters<runbooks::ListRunbooksParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(runbooks::handle_list_runbooks(self, params.0).await)
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
        Ok(runbooks::handle_search_runbooks(self, params.0).await)
    }

    #[tool(
        name = "create_runbook",
        description = "Create a new runbook with title, slug, content, tags, and metadata"
    )]
    async fn create_runbook(
        &self,
        params: Parameters<runbooks::CreateRunbookParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(runbooks::handle_create_runbook(self, params.0).await)
    }

    #[tool(
        name = "update_runbook",
        description = "Update an existing runbook by slug. Only provided fields are updated; version is auto-incremented."
    )]
    async fn update_runbook(
        &self,
        params: Parameters<runbooks::UpdateRunbookParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(runbooks::handle_update_runbook(self, params.0).await)
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
        Ok(knowledge::handle_add_knowledge(self, params.0).await)
    }

    #[tool(
        name = "update_knowledge",
        description = "Update an existing knowledge base entry by ID. Only provided fields are updated."
    )]
    async fn update_knowledge(
        &self,
        params: Parameters<knowledge::UpdateKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_update_knowledge(self, params.0).await)
    }

    #[tool(
        name = "delete_knowledge",
        description = "Delete a knowledge base entry by ID. Use with caution — this is permanent."
    )]
    async fn delete_knowledge(
        &self,
        params: Parameters<knowledge::DeleteKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_delete_knowledge(self, params.0).await)
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
        Ok(knowledge::handle_search_knowledge(self, params.0).await)
    }

    #[tool(
        name = "list_knowledge",
        description = "List knowledge base entries, optionally filtered by category or client"
    )]
    async fn list_knowledge(
        &self,
        params: Parameters<knowledge::ListKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_list_knowledge(self, params.0).await)
    }

    // ===== CONTEXT TOOLS =====

    #[tool(
        name = "get_situational_awareness",
        description = "THE KEY TOOL: Get comprehensive situational awareness for a server, service, or client. \
        Gathers all related data: entity details, related entities, recent incidents, pending handoffs, \
        relevant runbooks, vendor contacts, and knowledge entries. Provide at least one of server_slug, \
        service_slug, or client_slug. Use compact=true to reduce response size (~94K→~10K) by stripping \
        content/body fields — use drill-down tools for full details. Use sections to limit which parts \
        are returned (e.g. [\"server\",\"services\",\"monitoring\"])."
    )]
    async fn get_situational_awareness(
        &self,
        params: Parameters<context::GetSituationalAwarenessParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(context::handle_get_situational_awareness(self, params.0).await)
    }

    #[tool(
        name = "get_client_overview",
        description = "Get a full client briefing: all sites, servers, services, networks, vendors, recent incidents, and pending handoffs"
    )]
    async fn get_client_overview(
        &self,
        params: Parameters<context::GetClientOverviewParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(context::handle_get_client_overview(self, params.0).await)
    }

    #[tool(
        name = "get_server_context",
        description = "Get everything about a specific server: details, services, site, networks, \
        recent incidents for this server, related runbooks, and vendor contacts. \
        Use compact=true to reduce response size. Use sections to limit which parts are returned."
    )]
    async fn get_server_context(
        &self,
        params: Parameters<context::GetServerContextParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(context::handle_get_server_context(self, params.0).await)
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
        Ok(incidents::handle_create_incident(self, params.0).await)
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
        Ok(incidents::handle_update_incident(self, params.0).await)
    }

    #[tool(
        name = "get_incident",
        description = "Get full details of an incident by ID, including linked servers, services, runbooks, and vendors"
    )]
    async fn get_incident(
        &self,
        params: Parameters<incidents::GetIncidentParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(incidents::handle_get_incident(self, params.0).await)
    }

    #[tool(
        name = "list_incidents",
        description = "List incidents with optional filters by client, status, and severity"
    )]
    async fn list_incidents(
        &self,
        params: Parameters<incidents::ListIncidentsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(incidents::handle_list_incidents(self, params.0).await)
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
        Ok(incidents::handle_search_incidents(self, params.0).await)
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
        Ok(incidents::handle_link_incident(self, params.0).await)
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
        Ok(coordination::handle_start_session(self, params.0).await)
    }

    #[tool(
        name = "end_session",
        description = "End a work session with an optional summary of what was accomplished"
    )]
    async fn end_session(
        &self,
        params: Parameters<coordination::EndSessionParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_end_session(self, params.0).await)
    }

    #[tool(
        name = "list_sessions",
        description = "List work sessions, optionally filtered by machine and active status"
    )]
    async fn list_sessions(
        &self,
        params: Parameters<coordination::ListSessionsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_list_sessions(self, params.0).await)
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
        Ok(coordination::handle_create_handoff(self, params.0).await)
    }

    #[tool(
        name = "accept_handoff",
        description = "Accept a pending handoff, marking it as in-progress on your machine"
    )]
    async fn accept_handoff(
        &self,
        params: Parameters<coordination::UpdateHandoffStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_accept_handoff(self, params.0).await)
    }

    #[tool(name = "complete_handoff", description = "Mark a handoff as completed")]
    async fn complete_handoff(
        &self,
        params: Parameters<coordination::UpdateHandoffStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_complete_handoff(self, params.0).await)
    }

    #[tool(
        name = "list_handoffs",
        description = "List handoffs with optional filters. Use status='pending' to see what needs attention."
    )]
    async fn list_handoffs(
        &self,
        params: Parameters<coordination::ListHandoffsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_list_handoffs(self, params.0).await)
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
        Ok(coordination::handle_search_handoffs(self, params.0).await)
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
        Ok(search::handle_semantic_search(self, params.0).await)
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
        Ok(search::handle_backfill_embeddings(self, params.0).await)
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
        Ok(monitoring::handle_list_monitors(self, params.0).await)
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
        Ok(monitoring::handle_get_monitor_status(self, params.0).await)
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
        Ok(monitoring::handle_get_monitoring_summary(self, _params.0).await)
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
        Ok(monitoring::handle_link_monitor(self, params.0).await)
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
        Ok(monitoring::handle_unlink_monitor(self, params.0).await)
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
        Ok(monitoring::handle_list_watchdog_incidents(self, params.0).await)
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
        Ok(zammad::handle_list_tickets(self, params.0).await)
    }

    #[tool(
        name = "get_ticket",
        description = "Get a Zammad ticket by ID with full article history (messages, notes, time accounting)."
    )]
    async fn get_ticket(
        &self,
        params: Parameters<zammad::GetTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_get_ticket(self, params.0).await)
    }

    #[tool(
        name = "create_ticket",
        description = "Create a new ticket in Zammad. Resolves client_slug to Zammad group/org/customer. Optionally links to an ops-brain incident."
    )]
    async fn create_ticket(
        &self,
        params: Parameters<zammad::CreateTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_create_ticket(self, params.0).await)
    }

    #[tool(
        name = "update_ticket",
        description = "Update a Zammad ticket's state, priority, or title."
    )]
    async fn update_ticket(
        &self,
        params: Parameters<zammad::UpdateTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_update_ticket(self, params.0).await)
    }

    #[tool(
        name = "add_ticket_note",
        description = "Add an internal note (or public reply) to a Zammad ticket. Supports time accounting."
    )]
    async fn add_ticket_note(
        &self,
        params: Parameters<zammad::AddTicketNoteParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_add_ticket_note(self, params.0).await)
    }

    #[tool(
        name = "search_tickets",
        description = "Search Zammad tickets using full-text search (Elasticsearch syntax). Examples: 'soporte-usuario', 'backup failed', 'title:servidor'."
    )]
    async fn search_tickets(
        &self,
        params: Parameters<zammad::SearchTicketsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_search_tickets(self, params.0).await)
    }

    #[tool(
        name = "link_ticket",
        description = "Link a Zammad ticket to ops-brain entities (incident, server, service). At least one entity must be provided."
    )]
    async fn link_ticket(
        &self,
        params: Parameters<zammad::LinkTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_link_ticket(self, params.0).await)
    }

    #[tool(
        name = "unlink_ticket",
        description = "Remove the link between a Zammad ticket and ops-brain entities."
    )]
    async fn unlink_ticket(
        &self,
        params: Parameters<zammad::UnlinkTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_unlink_ticket(self, params.0).await)
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
        Ok(briefings::handle_generate_briefing(self, params.0).await)
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
        Ok(briefings::handle_list_briefings(self, params.0).await)
    }

    #[tool(
        name = "get_briefing",
        description = "Retrieve a specific briefing by ID."
    )]
    async fn get_briefing(
        &self,
        params: Parameters<briefings::GetBriefingParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(briefings::handle_get_briefing(self, params.0).await)
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
        Ok(inventory::handle_delete_server(self, params.0).await)
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
        Ok(inventory::handle_delete_service(self, params.0).await)
    }

    #[tool(
        name = "delete_vendor",
        description = "Delete a vendor by name or ID. Use ID to target a specific vendor when duplicates exist. \
        Without confirm=true, returns a preview of linked entities. Client and incident links are cascade-deleted."
    )]
    async fn delete_vendor(
        &self,
        params: Parameters<inventory::DeleteVendorParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(inventory::handle_delete_vendor(self, params.0).await)
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
                 server, service, or client. Use search_inventory for full-text search \
                 across all entity types (servers, services, vendors, clients, sites, networks, \
                 runbooks, knowledge, incidents, handoffs). \
                 Use semantic_search for AI-powered conceptual search across runbooks, \
                 knowledge, incidents, and handoffs (finds related content even without \
                 exact keyword matches). Use get_monitoring_summary for live infrastructure \
                 health from Uptime Kuma. Use list_tickets, search_tickets, and get_ticket \
                 for Zammad ticketing integration — create_ticket and add_ticket_note for \
                 ticket management with time accounting. Use generate_briefing for \
                 daily/weekly operational summaries aggregating monitoring, incidents, \
                 handoffs, and tickets. \
                 \
                 IMPORTANT: You are part of a multi-CC team. On session start, run these \
                 three searches in order: (1) search_knowledge query 'CC Team Identity Naming' \
                 for your identity and role, (2) search_knowledge query 'CC Team Compliance \
                 Data Sharing Coordination' for compliance rules and handoff protocol, \
                 (3) search_knowledge query 'CC Team Contribution Standards Session Protocol' \
                 for session startup/close checklists and quality standards. Follow the Session \
                 Startup Checklist before doing any other work. Always add_knowledge for gotchas \
                 and lessons learned, create_handoff when another CC should pick up work, and \
                 use get_situational_awareness before making infrastructure changes.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::helpers::*;
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
        assert_eq!(result.allowed[0]["_client_slug"], "hsr");
        assert_eq!(result.allowed[0]["_client_name"], "Hospice");
    }

    #[test]
    fn filter_cross_client_safe_allowed() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(hsr_id), true)];

        let result = filter_cross_client(items, "runbook", Some(cpa_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].0, item_id);
        assert_eq!(result.audit_entries[0].1, Some(hsr_id));
        assert_eq!(result.audit_entries[0].2, "released_safe");
    }

    #[test]
    fn filter_cross_client_acknowledged_released() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(hsr_id), false)];

        let result = filter_cross_client(items, "runbook", Some(cpa_id), true, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "released");
    }

    #[test]
    fn filter_cross_client_withheld() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(hsr_id), false)];

        let result = filter_cross_client(items, "runbook", Some(cpa_id), false, &lookup);

        assert!(result.allowed.is_empty());
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["count"], 1);
        assert_eq!(result.withheld_notices[0]["owning_client_slug"], "hsr");
        assert_eq!(result.withheld_notices[0]["entity_type"], "runbook");
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
        assert_eq!(result.audit_entries.len(), 2);
    }

    // ===== incident cross-client gating tests =====

    #[test]
    fn filter_incident_cross_client_withheld() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(hsr_id), false)];

        let result = filter_cross_client(items, "incident", Some(cpa_id), false, &lookup);

        assert!(result.allowed.is_empty());
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["entity_type"], "incident");
        assert_eq!(result.withheld_notices[0]["owning_client_slug"], "hsr");
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "withheld");
    }

    #[test]
    fn filter_incident_cross_client_safe_allowed() {
        let (hsr_id, cpa_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(hsr_id), true)];

        let result = filter_cross_client(items, "incident", Some(cpa_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "released_safe");
    }

    #[test]
    fn filter_incident_same_client_allowed() {
        let (hsr_id, _, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(hsr_id), false)];

        let result = filter_cross_client(items, "incident", Some(hsr_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert!(result.audit_entries.is_empty());
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

        assert!(item.get("_client_slug").is_none());
    }

    // ===== compact mode tests =====

    #[test]
    fn compact_runbook_strips_content() {
        let runbook = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
            "title": "Reboot procedure",
            "slug": "reboot",
            "category": "ops",
            "content": "Very long content that should be stripped in compact mode...",
            "client_id": Uuid::now_v7().to_string(),
            "cross_client_safe": false,
            "created_at": "2026-03-26T00:00:00Z",
            "updated_at": "2026-03-26T00:00:00Z",
        });

        let compacted = compact_value(&runbook, "runbook");
        assert!(compacted.get("id").is_some());
        assert!(compacted.get("title").is_some());
        assert!(compacted.get("slug").is_some());
        assert!(compacted.get("category").is_some());
        assert!(compacted.get("content").is_none());
        assert!(compacted.get("created_at").is_none());
    }

    #[test]
    fn compact_incident_keeps_key_fields() {
        let incident = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
            "title": "Server down",
            "severity": "critical",
            "status": "open",
            "client_id": Uuid::now_v7().to_string(),
            "reported_at": "2026-03-26T00:00:00Z",
            "symptoms": "Long symptoms text...",
            "root_cause": "Long root cause text...",
            "resolution": "Long resolution text...",
            "notes": "Long notes...",
        });

        let compacted = compact_value(&incident, "incident");
        assert!(compacted.get("title").is_some());
        assert!(compacted.get("severity").is_some());
        assert!(compacted.get("status").is_some());
        assert!(compacted.get("symptoms").is_none());
        assert!(compacted.get("root_cause").is_none());
        assert!(compacted.get("resolution").is_none());
        assert!(compacted.get("notes").is_none());
    }

    #[test]
    fn compact_knowledge_strips_content() {
        let knowledge = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
            "title": "DNS gotcha",
            "category": "networking",
            "content": "Very long knowledge content...",
            "client_id": null,
        });

        let compacted = compact_value(&knowledge, "knowledge");
        assert!(compacted.get("title").is_some());
        assert!(compacted.get("category").is_some());
        assert!(compacted.get("content").is_none());
    }

    #[test]
    fn compact_vec_applies_to_all() {
        let items = vec![
            serde_json::json!({"id": "1", "title": "A", "content": "long"}),
            serde_json::json!({"id": "2", "title": "B", "content": "long"}),
        ];
        let compacted = compact_vec(&items, "knowledge");
        assert_eq!(compacted.len(), 2);
        for item in &compacted {
            assert!(item.get("content").is_none());
        }
    }

    #[test]
    fn compact_non_object_returns_clone() {
        let val = serde_json::json!("just a string");
        let compacted = compact_value(&val, "runbook");
        assert_eq!(compacted, val);
    }

    #[test]
    fn section_included_none_means_all() {
        assert!(section_included(&None, "server"));
        assert!(section_included(&None, "anything"));
    }

    #[test]
    fn section_included_filters() {
        let sections = Some(vec!["server".to_string(), "monitoring".to_string()]);
        assert!(section_included(&sections, "server"));
        assert!(section_included(&sections, "monitoring"));
        assert!(!section_included(&sections, "knowledge"));
        assert!(!section_included(&sections, "runbooks"));
    }
}
