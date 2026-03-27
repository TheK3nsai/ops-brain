use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::helpers::{
    compact_value, compact_vec, error_result, filter_cross_client, json_result,
    not_found_with_suggestions, section_included,
};
use super::shared::{build_client_lookup, get_query_embedding, log_audit_entries};
use crate::models::handoff::Handoff;
use crate::models::incident::Incident;
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetSituationalAwarenessParams {
    /// Server slug to get context for
    pub server_slug: Option<String>,
    /// Service slug to get context for
    pub service_slug: Option<String>,
    /// Client slug to get context for
    pub client_slug: Option<String>,
    /// Set to true to release cross-client runbooks/knowledge that were withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
    /// Compact mode: strip heavy fields (content, body, notes) from results, keeping only
    /// identifiers and key metadata. Reduces response from ~94K to ~10K chars. Use drill-down
    /// tools (get_runbook, get_incident, etc.) for full details. Default: false
    pub compact: Option<bool>,
    /// Limit response to specific sections. Valid values: server, site, client, services,
    /// networks, vendors, incidents, runbooks, handoffs, knowledge, monitoring, tickets.
    /// If omitted, all sections are included.
    pub sections: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetClientOverviewParams {
    pub client_slug: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetServerContextParams {
    pub server_slug: String,
    /// Set to true to release cross-client runbooks/knowledge that were withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
    /// Compact mode: strip heavy fields (content, body, notes) from results. Default: false
    pub compact: Option<bool>,
    /// Limit response to specific sections. Valid values: server, site, client, services,
    /// networks, vendors, incidents, runbooks, knowledge, monitoring, tickets.
    /// If omitted, all sections are included.
    pub sections: Option<Vec<String>>,
}

// Response structures for context tools
#[derive(Debug, Serialize)]
pub struct SituationalAwareness {
    pub server: Option<serde_json::Value>,
    pub site: Option<serde_json::Value>,
    pub client: Option<serde_json::Value>,
    pub services: Vec<serde_json::Value>,
    pub networks: Vec<serde_json::Value>,
    pub vendors: Vec<serde_json::Value>,
    pub recent_incidents: Vec<serde_json::Value>,
    pub relevant_runbooks: Vec<serde_json::Value>,
    pub pending_handoffs: Vec<serde_json::Value>,
    pub knowledge: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub monitoring: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub linked_tickets: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub cross_client_withheld: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub _warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct ClientOverview {
    pub client: serde_json::Value,
    pub sites: Vec<serde_json::Value>,
    pub servers: Vec<serde_json::Value>,
    pub services: Vec<serde_json::Value>,
    pub networks: Vec<serde_json::Value>,
    pub vendors: Vec<serde_json::Value>,
    pub recent_incidents: Vec<serde_json::Value>,
    pub pending_handoffs: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub recent_tickets: Vec<serde_json::Value>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub _warnings: Vec<String>,
}

// ===== HANDLERS =====

pub(crate) async fn handle_get_situational_awareness(
    brain: &super::OpsBrain,
    p: GetSituationalAwarenessParams,
) -> CallToolResult {
    if p.server_slug.is_none() && p.service_slug.is_none() && p.client_slug.is_none() {
        return error_result("Provide at least one of: server_slug, service_slug, or client_slug");
    }

    let compact = p.compact.unwrap_or(false);
    let sections = p.sections;
    let acknowledge = p.acknowledge_cross_client.unwrap_or(false);

    let mut warnings: Vec<String> = Vec::new();
    let mut awareness = SituationalAwareness {
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
        _warnings: Vec::new(),
    };

    let mut client_id: Option<uuid::Uuid> = None;
    #[allow(unused_assignments)]
    let mut site_id: Option<uuid::Uuid> = None;
    let mut server_id: Option<uuid::Uuid> = None;
    let mut service_id: Option<uuid::Uuid> = None;

    // Resolve server if provided — this gives us site and client context too
    if let Some(slug) = &p.server_slug {
        if let Ok(Some(server)) =
            crate::repo::server_repo::get_server_by_slug(&brain.pool, slug).await
        {
            server_id = Some(server.id);
            site_id = Some(server.site_id);
            awareness.server = serde_json::to_value(&server).ok();

            // Get services for this server
            if let Ok(services) =
                crate::repo::service_repo::get_services_for_server(&brain.pool, server.id).await
            {
                awareness.services = services
                    .iter()
                    .filter_map(|s| serde_json::to_value(s).ok())
                    .collect();
            }

            // Get site
            if let Ok(Some(site)) =
                crate::repo::site_repo::get_site(&brain.pool, server.site_id).await
            {
                client_id = Some(site.client_id);
                awareness.site = serde_json::to_value(&site).ok();

                // Get client from site
                if let Ok(Some(client)) =
                    crate::repo::client_repo::get_client(&brain.pool, site.client_id).await
                {
                    awareness.client = serde_json::to_value(&client).ok();
                }
            }

            // Get networks for this site
            if let Some(sid) = site_id {
                if let Ok(networks) =
                    crate::repo::network_repo::list_networks(&brain.pool, Some(sid)).await
                {
                    awareness.networks = networks
                        .iter()
                        .filter_map(|n| serde_json::to_value(n).ok())
                        .collect();
                }
            }

            // Get runbooks linked to this server
            if let Ok(runbooks) = crate::repo::runbook_repo::list_runbooks(
                &brain.pool,
                None,
                None,
                Some(server.id),
                None,
                None,
                200,
            )
            .await
            {
                awareness.relevant_runbooks = runbooks
                    .iter()
                    .filter_map(|r| serde_json::to_value(r).ok())
                    .collect();
            }
        } else {
            return not_found_with_suggestions(&brain.pool, "Server", slug).await;
        }
    }

    // Resolve service if provided
    if let Some(slug) = &p.service_slug {
        if let Ok(Some(svc)) =
            crate::repo::service_repo::get_service_by_slug(&brain.pool, slug).await
        {
            service_id = Some(svc.id);

            // Add service to list if not already present from server lookup
            if awareness.services.is_empty() {
                awareness.services =
                    vec![serde_json::to_value(&svc).unwrap_or(serde_json::Value::Null)];
            }

            // Get servers running this service
            if let Ok(servers) =
                crate::repo::service_repo::get_servers_for_service(&brain.pool, svc.id).await
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
                            crate::repo::site_repo::get_site(&brain.pool, first_server.site_id)
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
                &brain.pool,
                None,
                Some(svc.id),
                None,
                None,
                None,
                200,
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
            return not_found_with_suggestions(&brain.pool, "Service", slug).await;
        }
    }

    // Resolve client if provided (may already be set from server/service lookup)
    if let Some(slug) = &p.client_slug {
        if let Ok(Some(client)) =
            crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await
        {
            client_id = Some(client.id);
            awareness.client = serde_json::to_value(&client).ok();

            // Client-only query: aggregate servers → services/networks from all sites
            if p.server_slug.is_none() && p.service_slug.is_none() {
                // Get all servers for this client
                if section_included(&sections, "services") || section_included(&sections, "server")
                {
                    if let Ok(servers) = crate::repo::server_repo::list_servers(
                        &brain.pool,
                        Some(client.id),
                        None,
                        None,
                        None,
                        200,
                    )
                    .await
                    {
                        // Collect services from all servers (deduplicated)
                        let mut seen_service_ids = std::collections::HashSet::new();
                        for srv in &servers {
                            if let Ok(svcs) = crate::repo::service_repo::get_services_for_server(
                                &brain.pool,
                                srv.id,
                            )
                            .await
                            {
                                for svc in svcs {
                                    if seen_service_ids.insert(svc.id) {
                                        if let Ok(val) = serde_json::to_value(&svc) {
                                            awareness.services.push(val);
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Get networks from all sites
                if section_included(&sections, "networks") {
                    if let Ok(sites) =
                        crate::repo::site_repo::list_sites(&brain.pool, Some(client.id)).await
                    {
                        for site in &sites {
                            if let Ok(nets) =
                                crate::repo::network_repo::list_networks(&brain.pool, Some(site.id))
                                    .await
                            {
                                for net in nets {
                                    if let Ok(val) = serde_json::to_value(&net) {
                                        awareness.networks.push(val);
                                    }
                                }
                            }
                        }
                    }
                }
            }
        } else {
            return not_found_with_suggestions(&brain.pool, "Client", slug).await;
        }
    }

    // Get vendors for client
    if let Some(cid) = client_id {
        match crate::repo::vendor_repo::get_vendors_for_client(&brain.pool, cid).await {
            Ok(vendors) => {
                awareness.vendors = vendors
                    .iter()
                    .filter_map(|v| serde_json::to_value(v).ok())
                    .collect();
            }
            Err(e) => warnings.push(format!("Vendor lookup failed: {e}")),
        }

        // Get knowledge for this client
        match crate::repo::knowledge_repo::list_knowledge(&brain.pool, None, Some(cid), 100).await {
            Ok(entries) => {
                awareness.knowledge = entries
                    .iter()
                    .filter_map(|k| serde_json::to_value(k).ok())
                    .collect();
            }
            Err(e) => warnings.push(format!("Knowledge lookup failed: {e}")),
        }
    }

    // Get all related incidents in a single UNION query (client + server + service)
    if section_included(&sections, "incidents") {
        match crate::repo::incident_repo::get_related_incidents(
            &brain.pool,
            client_id,
            server_id,
            service_id,
            10,
        )
        .await
        {
            Ok(incidents) => {
                awareness.recent_incidents = incidents
                    .iter()
                    .filter_map(|i| serde_json::to_value(i).ok())
                    .collect();
            }
            Err(e) => warnings.push(format!("Incident lookup failed: {e}")),
        }
    }

    // Get pending handoffs
    match sqlx::query_as::<_, Handoff>(
        "SELECT * FROM handoffs WHERE status = 'pending' ORDER BY created_at DESC LIMIT 10",
    )
    .fetch_all(&brain.pool)
    .await
    {
        Ok(handoffs) => {
            awareness.pending_handoffs = handoffs
                .iter()
                .filter_map(|h| serde_json::to_value(h).ok())
                .collect();
        }
        Err(e) => warnings.push(format!("Handoff lookup failed: {e}")),
    }

    // Also add general knowledge (not client-specific)
    match crate::repo::knowledge_repo::list_knowledge(&brain.pool, None, None, 100).await {
        Ok(general_knowledge) => {
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
        Err(e) => warnings.push(format!("General knowledge lookup failed: {e}")),
    }

    // Semantic enrichment: find related runbooks/knowledge beyond explicit links
    if brain.embedding_client.is_some() {
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
            if let Some(emb) = get_query_embedding(&brain.embedding_client, &context_query).await {
                // Find semantically related runbooks
                if let Ok(related_runbooks) =
                    crate::repo::embedding_repo::vector_search_runbooks(&brain.pool, &emb, 5).await
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
                    crate::repo::embedding_repo::vector_search_knowledge(&brain.pool, &emb, 5).await
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

    // Cross-client scope gate for runbooks, knowledge, and incidents
    {
        let client_lookup = build_client_lookup(&brain.pool).await;

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
        log_audit_entries(
            &brain.pool,
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
        log_audit_entries(
            &brain.pool,
            "get_situational_awareness",
            client_id,
            "knowledge",
            &kn_filtered.audit_entries,
        )
        .await;

        let inc_filtered = filter_cross_client(
            std::mem::take(&mut awareness.recent_incidents),
            "incident",
            client_id,
            acknowledge,
            &client_lookup,
        );
        awareness.recent_incidents = inc_filtered.allowed;
        awareness
            .cross_client_withheld
            .extend(inc_filtered.withheld_notices);
        log_audit_entries(
            &brain.pool,
            "get_situational_awareness",
            client_id,
            "incident",
            &inc_filtered.audit_entries,
        )
        .await;
    }

    // Fetch live monitoring data for linked servers/services
    if let Some(ref kuma_config) = brain.kuma_config {
        match crate::metrics::fetch_metrics(kuma_config).await {
            Ok(metrics) => {
                // Get monitor mappings for this server and its services
                let mut monitor_names: std::collections::HashSet<String> =
                    std::collections::HashSet::new();

                if let Some(srv_id) = server_id {
                    if let Ok(monitors) =
                        crate::repo::monitor_repo::get_monitors_for_server(&brain.pool, srv_id)
                            .await
                    {
                        for m in &monitors {
                            monitor_names.insert(m.monitor_name.clone());
                        }
                    }
                }

                if let Some(svc_id) = service_id {
                    if let Ok(monitors) =
                        crate::repo::monitor_repo::get_monitors_for_service(&brain.pool, svc_id)
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
            Err(e) => warnings.push(format!("Uptime Kuma unreachable: {e}")),
        }
    }

    // Zammad linked tickets for this server/service
    if brain.zammad_config.is_some() {
        if let Some(srv_id) = server_id {
            match crate::repo::ticket_link_repo::get_links_for_server(&brain.pool, srv_id).await {
                Ok(links) => {
                    for link in &links {
                        if let Ok(val) = serde_json::to_value(link) {
                            awareness.linked_tickets.push(val);
                        }
                    }
                }
                Err(e) => warnings.push(format!("Ticket links (server) failed: {e}")),
            }
        }
        if let Some(svc_id) = service_id {
            match crate::repo::ticket_link_repo::get_links_for_service(&brain.pool, svc_id).await {
                Ok(links) => {
                    for link in &links {
                        if let Ok(val) = serde_json::to_value(link) {
                            awareness.linked_tickets.push(val);
                        }
                    }
                }
                Err(e) => warnings.push(format!("Ticket links (service) failed: {e}")),
            }
        }
    }

    // Apply compact mode and sections filtering
    if compact || sections.is_some() {
        if compact {
            if let Some(ref v) = awareness.server {
                awareness.server = Some(compact_value(v, "server"));
            }
            if let Some(ref v) = awareness.site {
                awareness.site = Some(compact_value(v, "site"));
            }
            if let Some(ref v) = awareness.client {
                awareness.client = Some(compact_value(v, "client"));
            }
            awareness.services = compact_vec(&awareness.services, "service");
            awareness.networks = compact_vec(&awareness.networks, "network");
            awareness.vendors = compact_vec(&awareness.vendors, "vendor");
            awareness.recent_incidents = compact_vec(&awareness.recent_incidents, "incident");
            awareness.relevant_runbooks = compact_vec(&awareness.relevant_runbooks, "runbook");
            awareness.pending_handoffs = compact_vec(&awareness.pending_handoffs, "handoff");
            awareness.knowledge = compact_vec(&awareness.knowledge, "knowledge");
            awareness.monitoring = compact_vec(&awareness.monitoring, "monitor");
            awareness.linked_tickets = compact_vec(&awareness.linked_tickets, "ticket");
        }
        if sections.is_some() {
            if !section_included(&sections, "server") {
                awareness.server = None;
            }
            if !section_included(&sections, "site") {
                awareness.site = None;
            }
            if !section_included(&sections, "client") {
                awareness.client = None;
            }
            if !section_included(&sections, "services") {
                awareness.services.clear();
            }
            if !section_included(&sections, "networks") {
                awareness.networks.clear();
            }
            if !section_included(&sections, "vendors") {
                awareness.vendors.clear();
            }
            if !section_included(&sections, "incidents") {
                awareness.recent_incidents.clear();
            }
            if !section_included(&sections, "runbooks") {
                awareness.relevant_runbooks.clear();
            }
            if !section_included(&sections, "handoffs") {
                awareness.pending_handoffs.clear();
            }
            if !section_included(&sections, "knowledge") {
                awareness.knowledge.clear();
            }
            if !section_included(&sections, "monitoring") {
                awareness.monitoring.clear();
            }
            if !section_included(&sections, "tickets") {
                awareness.linked_tickets.clear();
            }
        }
    }

    awareness._warnings = warnings;
    json_result(&awareness)
}

pub(crate) async fn handle_get_client_overview(
    brain: &super::OpsBrain,
    p: GetClientOverviewParams,
) -> CallToolResult {
    let client = match crate::repo::client_repo::get_client_by_slug(&brain.pool, &p.client_slug)
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", &p.client_slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    let mut warnings: Vec<String> = Vec::new();

    let sites = crate::repo::site_repo::list_sites(&brain.pool, Some(client.id))
        .await
        .unwrap_or_else(|e| {
            warnings.push(format!("Sites lookup failed: {e}"));
            Vec::new()
        });

    let servers =
        crate::repo::server_repo::list_servers(&brain.pool, Some(client.id), None, None, None, 200)
            .await
            .unwrap_or_else(|e| {
                warnings.push(format!("Servers lookup failed: {e}"));
                Vec::new()
            });

    // Collect all service IDs from all servers
    let mut all_services = Vec::new();
    let mut seen_service_ids = std::collections::HashSet::new();
    for server in &servers {
        if let Ok(svcs) =
            crate::repo::service_repo::get_services_for_server(&brain.pool, server.id).await
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
        if let Ok(nets) = crate::repo::network_repo::list_networks(&brain.pool, Some(site.id)).await
        {
            all_networks.extend(nets);
        }
    }

    let vendors = crate::repo::vendor_repo::get_vendors_for_client(&brain.pool, client.id)
        .await
        .unwrap_or_else(|e| {
            warnings.push(format!("Vendor lookup failed: {e}"));
            Vec::new()
        });

    let recent_incidents: Vec<Incident> = sqlx::query_as::<_, Incident>(
        "SELECT * FROM incidents WHERE client_id = $1 ORDER BY reported_at DESC LIMIT 10",
    )
    .bind(client.id)
    .fetch_all(&brain.pool)
    .await
    .unwrap_or_else(|e| {
        warnings.push(format!("Incident lookup failed: {e}"));
        Vec::new()
    });

    let pending_handoffs: Vec<Handoff> = sqlx::query_as::<_, Handoff>(
        "SELECT * FROM handoffs WHERE status = 'pending' ORDER BY created_at DESC LIMIT 10",
    )
    .fetch_all(&brain.pool)
    .await
    .unwrap_or_else(|e| {
        warnings.push(format!("Handoff lookup failed: {e}"));
        Vec::new()
    });

    let mut overview = ClientOverview {
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
        _warnings: Vec::new(),
    };

    // Fetch recent Zammad tickets for this client
    if let Some(ref zammad) = brain.zammad_config {
        if let Some(org_id) = client.zammad_org_id {
            let query = format!("organization.id:{org_id}");
            match crate::zammad::search_tickets(zammad, &query, 5).await {
                Ok(tickets) => {
                    overview.recent_tickets = tickets
                        .iter()
                        .filter_map(|t| serde_json::to_value(t).ok())
                        .collect();
                }
                Err(e) => warnings.push(format!("Zammad ticket search failed: {e}")),
            }
        }
    }

    overview._warnings = warnings;
    json_result(&overview)
}

pub(crate) async fn handle_get_server_context(
    brain: &super::OpsBrain,
    p: GetServerContextParams,
) -> CallToolResult {
    let acknowledge = p.acknowledge_cross_client.unwrap_or(false);
    let compact = p.compact.unwrap_or(false);
    let sections = p.sections;
    let mut warnings: Vec<String> = Vec::new();

    let server = match crate::repo::server_repo::get_server_by_slug(&brain.pool, &p.server_slug)
        .await
    {
        Ok(Some(s)) => s,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Server", &p.server_slug).await,
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

    // Get client for vendor lookup
    let client_id = site.as_ref().map(|s| s.client_id);
    let client = if let Some(cid) = client_id {
        crate::repo::client_repo::get_client(&brain.pool, cid)
            .await
            .ok()
            .flatten()
    } else {
        None
    };

    let vendors = if let Some(cid) = client_id {
        crate::repo::vendor_repo::get_vendors_for_client(&brain.pool, cid)
            .await
            .unwrap_or_else(|e| {
                warnings.push(format!("Vendor lookup failed: {e}"));
                Vec::new()
            })
    } else {
        Vec::new()
    };

    // Get all related incidents in a single UNION query (client + server)
    let mut all_incidents: Vec<serde_json::Value> =
        match crate::repo::incident_repo::get_related_incidents(
            &brain.pool,
            client_id,
            Some(server.id),
            None,
            10,
        )
        .await
        {
            Ok(incidents) => incidents
                .iter()
                .filter_map(|i| serde_json::to_value(i).ok())
                .collect(),
            Err(e) => {
                warnings.push(format!("Incident lookup failed: {e}"));
                Vec::new()
            }
        };

    // Get runbooks linked to this server
    let runbooks = crate::repo::runbook_repo::list_runbooks(
        &brain.pool,
        None,
        None,
        Some(server.id),
        None,
        None,
        200,
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
            &brain.pool,
            None,
            Some(svc.id),
            None,
            None,
            None,
            200,
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
        match crate::repo::knowledge_repo::list_knowledge(&brain.pool, None, Some(cid), 100).await {
            Ok(entries) => entries
                .iter()
                .filter_map(|k| serde_json::to_value(k).ok())
                .collect(),
            Err(e) => {
                warnings.push(format!("Knowledge lookup failed: {e}"));
                Vec::new()
            }
        }
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
    if brain.embedding_client.is_some() {
        let mut context_parts = vec![server.hostname.clone()];
        if let Some(ref os) = server.os {
            context_parts.push(os.clone());
        }
        for svc in &services {
            context_parts.push(svc.name.clone());
        }
        let context_query = context_parts.join(" ");
        if let Some(emb) = get_query_embedding(&brain.embedding_client, &context_query).await {
            if let Ok(related_runbooks) =
                crate::repo::embedding_repo::vector_search_runbooks(&brain.pool, &emb, 5).await
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
                crate::repo::embedding_repo::vector_search_knowledge(&brain.pool, &emb, 5).await
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

    // Cross-client scope gate for runbooks, knowledge, and incidents
    let mut cross_client_withheld: Vec<serde_json::Value> = Vec::new();
    {
        let client_lookup = build_client_lookup(&brain.pool).await;

        let rb_filtered = filter_cross_client(
            std::mem::take(&mut all_runbooks),
            "runbook",
            client_id,
            acknowledge,
            &client_lookup,
        );
        all_runbooks = rb_filtered.allowed;
        cross_client_withheld.extend(rb_filtered.withheld_notices);
        log_audit_entries(
            &brain.pool,
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
        log_audit_entries(
            &brain.pool,
            "get_server_context",
            client_id,
            "knowledge",
            &kn_filtered.audit_entries,
        )
        .await;

        let inc_filtered = filter_cross_client(
            std::mem::take(&mut all_incidents),
            "incident",
            client_id,
            acknowledge,
            &client_lookup,
        );
        all_incidents = inc_filtered.allowed;
        cross_client_withheld.extend(inc_filtered.withheld_notices);
        log_audit_entries(
            &brain.pool,
            "get_server_context",
            client_id,
            "incident",
            &inc_filtered.audit_entries,
        )
        .await;
    }

    // Fetch live monitoring data for this server and its services
    let mut monitoring: Vec<serde_json::Value> = Vec::new();
    if let Some(ref kuma_config) = brain.kuma_config {
        match crate::metrics::fetch_metrics(kuma_config).await {
            Ok(metrics) => {
                let mut monitor_names: std::collections::HashSet<String> =
                    std::collections::HashSet::new();

                if let Ok(monitors) =
                    crate::repo::monitor_repo::get_monitors_for_server(&brain.pool, server.id).await
                {
                    for m in &monitors {
                        monitor_names.insert(m.monitor_name.clone());
                    }
                }

                for svc in &services {
                    if let Ok(monitors) =
                        crate::repo::monitor_repo::get_monitors_for_service(&brain.pool, svc.id)
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
            Err(e) => warnings.push(format!("Uptime Kuma unreachable: {e}")),
        }
    }

    // Zammad linked tickets for this server
    let mut linked_tickets: Vec<serde_json::Value> = Vec::new();
    if brain.zammad_config.is_some() {
        match crate::repo::ticket_link_repo::get_links_for_server(&brain.pool, server.id).await {
            Ok(links) => {
                for link in &links {
                    if let Ok(val) = serde_json::to_value(link) {
                        linked_tickets.push(val);
                    }
                }
            }
            Err(e) => warnings.push(format!("Ticket links failed: {e}")),
        }
    }

    let server_json = serde_json::to_value(&server).unwrap_or_default();
    let site_json = serde_json::to_value(&site).unwrap_or_default();
    let client_json = serde_json::to_value(&client).unwrap_or_default();
    let services_json: Vec<serde_json::Value> = services
        .iter()
        .filter_map(|s| serde_json::to_value(s).ok())
        .collect();
    let networks_json: Vec<serde_json::Value> = networks
        .iter()
        .filter_map(|n| serde_json::to_value(n).ok())
        .collect();
    let vendors_json: Vec<serde_json::Value> = vendors
        .iter()
        .filter_map(|v| serde_json::to_value(v).ok())
        .collect();

    let mut result = serde_json::json!({});

    if section_included(&sections, "server") {
        result["server"] = if compact {
            compact_value(&server_json, "server")
        } else {
            server_json
        };
    }
    if section_included(&sections, "services") {
        result["services"] = if compact {
            serde_json::to_value(compact_vec(&services_json, "service")).unwrap_or_default()
        } else {
            serde_json::to_value(&services_json).unwrap_or_default()
        };
    }
    if section_included(&sections, "site") {
        result["site"] = if compact {
            compact_value(&site_json, "site")
        } else {
            site_json
        };
    }
    if section_included(&sections, "client") {
        result["client"] = if compact {
            compact_value(&client_json, "client")
        } else {
            client_json
        };
    }
    if section_included(&sections, "networks") {
        result["networks"] = if compact {
            serde_json::to_value(compact_vec(&networks_json, "network")).unwrap_or_default()
        } else {
            serde_json::to_value(&networks_json).unwrap_or_default()
        };
    }
    if section_included(&sections, "vendors") {
        result["vendors"] = if compact {
            serde_json::to_value(compact_vec(&vendors_json, "vendor")).unwrap_or_default()
        } else {
            serde_json::to_value(&vendors_json).unwrap_or_default()
        };
    }
    if section_included(&sections, "incidents") {
        result["recent_incidents"] = if compact {
            serde_json::to_value(compact_vec(&all_incidents, "incident")).unwrap_or_default()
        } else {
            serde_json::to_value(&all_incidents).unwrap_or_default()
        };
    }
    if section_included(&sections, "runbooks") {
        result["runbooks"] = if compact {
            serde_json::to_value(compact_vec(&all_runbooks, "runbook")).unwrap_or_default()
        } else {
            serde_json::to_value(&all_runbooks).unwrap_or_default()
        };
    }
    if section_included(&sections, "knowledge") {
        result["knowledge"] = if compact {
            serde_json::to_value(compact_vec(&all_knowledge, "knowledge")).unwrap_or_default()
        } else {
            serde_json::to_value(&all_knowledge).unwrap_or_default()
        };
    }
    if section_included(&sections, "monitoring") && !monitoring.is_empty() {
        result["monitoring"] = if compact {
            serde_json::to_value(compact_vec(&monitoring, "monitor")).unwrap_or_default()
        } else {
            serde_json::to_value(&monitoring).unwrap_or_default()
        };
    }
    if section_included(&sections, "tickets") && !linked_tickets.is_empty() {
        result["linked_tickets"] = if compact {
            serde_json::to_value(compact_vec(&linked_tickets, "ticket")).unwrap_or_default()
        } else {
            serde_json::to_value(&linked_tickets).unwrap_or_default()
        };
    }
    if !cross_client_withheld.is_empty() {
        result["cross_client_withheld"] = serde_json::json!(cross_client_withheld);
    }
    if !warnings.is_empty() {
        result["_warnings"] = serde_json::json!(warnings);
    }

    json_result(&result)
}
