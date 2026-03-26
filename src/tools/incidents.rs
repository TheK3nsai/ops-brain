use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{
    error_result, json_result, not_found, not_found_vendor_with_suggestions,
    not_found_with_suggestions,
};
use super::shared::{embed_and_store, get_query_embedding};
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateIncidentParams {
    /// Short title describing the incident
    pub title: String,
    /// Severity: low, medium, high, or critical
    pub severity: Option<String>,
    /// Client slug this incident belongs to
    pub client_slug: Option<String>,
    /// Initial symptoms observed
    pub symptoms: Option<String>,
    /// Any initial notes
    pub notes: Option<String>,
    /// Server slugs affected by this incident
    pub server_slugs: Option<Vec<String>>,
    /// Service slugs affected by this incident
    pub service_slugs: Option<Vec<String>>,
    /// Mark as safe to surface in cross-client contexts (default: false)
    pub cross_client_safe: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateIncidentParams {
    /// Incident ID (UUID)
    pub id: String,
    /// Updated title
    pub title: Option<String>,
    /// Updated status: open or resolved
    pub status: Option<String>,
    /// Updated severity: low, medium, high, or critical
    pub severity: Option<String>,
    /// Symptoms description
    pub symptoms: Option<String>,
    /// Root cause analysis
    pub root_cause: Option<String>,
    /// How it was resolved
    pub resolution: Option<String>,
    /// Steps to prevent recurrence
    pub prevention: Option<String>,
    /// Additional notes
    pub notes: Option<String>,
    /// Mark as safe to surface in cross-client contexts
    pub cross_client_safe: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetIncidentParams {
    /// Incident ID (UUID)
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListIncidentsParams {
    /// Filter by client slug
    pub client_slug: Option<String>,
    /// Filter by status: open or resolved
    pub status: Option<String>,
    /// Filter by severity: low, medium, high, or critical
    pub severity: Option<String>,
    /// Max results (default 20)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchIncidentsParams {
    /// Full-text search query
    pub query: String,
    /// Search mode: "fts" (default), "semantic" (vector only), or "hybrid" (FTS + vector RRF)
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkIncidentParams {
    /// Incident ID (UUID)
    pub incident_id: String,
    /// Server slugs to link
    pub server_slugs: Option<Vec<String>>,
    /// Service slugs to link
    pub service_slugs: Option<Vec<String>>,
    /// Runbook slugs to link, with usage type
    pub runbook_links: Option<Vec<RunbookLink>>,
    /// Vendor names to link
    pub vendor_names: Option<Vec<String>>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct RunbookLink {
    /// Runbook slug
    pub slug: String,
    /// Usage: followed, not-applicable, or not-followed
    pub usage: Option<String>,
}

// ===== HANDLERS =====

pub(crate) async fn handle_create_incident(
    brain: &super::OpsBrain,
    p: CreateIncidentParams,
) -> CallToolResult {
    let severity = p.severity.as_deref().unwrap_or("medium");

    if let Err(msg) = crate::validation::validate_required(
        severity,
        "severity",
        crate::validation::INCIDENT_SEVERITIES,
    ) {
        return error_result(&msg);
    }

    // Resolve client_slug
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    let cross_client_safe = p.cross_client_safe.unwrap_or(false);
    let incident = match crate::repo::incident_repo::create_incident(
        &brain.pool,
        &p.title,
        severity,
        client_id,
        p.symptoms.as_deref(),
        p.notes.as_deref(),
        cross_client_safe,
    )
    .await
    {
        Ok(i) => i,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    // Link servers if provided
    if let Some(slugs) = &p.server_slugs {
        for slug in slugs {
            if let Ok(Some(server)) =
                crate::repo::server_repo::get_server_by_slug(&brain.pool, slug).await
            {
                let _ = crate::repo::incident_repo::link_incident_server(
                    &brain.pool,
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
                crate::repo::service_repo::get_service_by_slug(&brain.pool, slug).await
            {
                let _ = crate::repo::incident_repo::link_incident_service(
                    &brain.pool,
                    incident.id,
                    service.id,
                )
                .await;
            }
        }
    }

    let text = crate::embeddings::prepare_incident_text(&incident);
    embed_and_store(
        &brain.pool,
        &brain.embedding_client,
        "incidents",
        incident.id,
        &text,
    )
    .await;

    json_result(&incident)
}

pub(crate) async fn handle_update_incident(
    brain: &super::OpsBrain,
    p: UpdateIncidentParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.id)),
    };

    if let Err(msg) = crate::validation::validate_option(
        p.status.as_deref(),
        "status",
        crate::validation::INCIDENT_STATUSES,
    ) {
        return error_result(&msg);
    }
    if let Err(msg) = crate::validation::validate_option(
        p.severity.as_deref(),
        "severity",
        crate::validation::INCIDENT_SEVERITIES,
    ) {
        return error_result(&msg);
    }

    match crate::repo::incident_repo::update_incident(
        &brain.pool,
        id,
        p.title.as_deref(),
        p.status.as_deref(),
        p.severity.as_deref(),
        p.symptoms.as_deref(),
        p.root_cause.as_deref(),
        p.resolution.as_deref(),
        p.prevention.as_deref(),
        p.notes.as_deref(),
        p.cross_client_safe,
    )
    .await
    {
        Ok(incident) => {
            let text = crate::embeddings::prepare_incident_text(&incident);
            embed_and_store(
                &brain.pool,
                &brain.embedding_client,
                "incidents",
                incident.id,
                &text,
            )
            .await;
            json_result(&incident)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_get_incident(
    brain: &super::OpsBrain,
    p: GetIncidentParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.id)),
    };

    let incident = match crate::repo::incident_repo::get_incident(&brain.pool, id).await {
        Ok(Some(i)) => i,
        Ok(None) => return not_found("Incident", &p.id),
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    // Get linked entities
    let linked_servers: Vec<crate::models::server::Server> = sqlx::query_as(
        "SELECT s.* FROM servers s JOIN incident_servers isv ON s.id = isv.server_id WHERE isv.incident_id = $1",
    )
    .bind(id)
    .fetch_all(&brain.pool)
    .await
    .unwrap_or_default();

    let linked_services: Vec<crate::models::service::Service> = sqlx::query_as(
        "SELECT s.* FROM services s JOIN incident_services iss ON s.id = iss.service_id WHERE iss.incident_id = $1",
    )
    .bind(id)
    .fetch_all(&brain.pool)
    .await
    .unwrap_or_default();

    let result = serde_json::json!({
        "incident": incident,
        "linked_servers": linked_servers,
        "linked_services": linked_services,
    });

    json_result(&result)
}

pub(crate) async fn handle_list_incidents(
    brain: &super::OpsBrain,
    p: ListIncidentsParams,
) -> CallToolResult {
    let limit = p.limit.unwrap_or(20);

    // Validate filters
    if let Err(msg) = crate::validation::validate_option(
        p.status.as_deref(),
        "status",
        crate::validation::INCIDENT_STATUSES,
    ) {
        return error_result(&msg);
    }
    if let Err(msg) = crate::validation::validate_option(
        p.severity.as_deref(),
        "severity",
        crate::validation::INCIDENT_SEVERITIES,
    ) {
        return error_result(&msg);
    }

    // Resolve client_slug
    let client_id = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    match crate::repo::incident_repo::list_incidents(
        &brain.pool,
        client_id,
        p.status.as_deref(),
        p.severity.as_deref(),
        limit,
    )
    .await
    {
        Ok(incidents) => json_result(&incidents),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_search_incidents(
    brain: &super::OpsBrain,
    p: SearchIncidentsParams,
) -> CallToolResult {
    let mode = p.mode.as_deref().unwrap_or("fts");
    if let Err(msg) =
        crate::validation::validate_required(mode, "mode", crate::validation::SEARCH_MODES)
    {
        return error_result(&msg);
    }
    let result = match mode {
        "semantic" => {
            let Some(emb) = get_query_embedding(&brain.embedding_client, &p.query).await else {
                return error_result("Semantic search unavailable (OPENAI_API_KEY not set)");
            };
            crate::repo::embedding_repo::vector_search_incidents(&brain.pool, &emb, 20).await
        }
        "hybrid" => {
            let emb = get_query_embedding(&brain.embedding_client, &p.query).await;
            crate::repo::embedding_repo::hybrid_search_incidents(
                &brain.pool,
                &p.query,
                emb.as_deref(),
                20,
            )
            .await
        }
        _ => crate::repo::incident_repo::search_incidents(&brain.pool, &p.query).await,
    };
    match result {
        Ok(incidents) => json_result(&incidents),
        Err(e) => error_result(&format!("Search error: {e}")),
    }
}

pub(crate) async fn handle_link_incident(
    brain: &super::OpsBrain,
    p: LinkIncidentParams,
) -> CallToolResult {
    let incident_id = match uuid::Uuid::parse_str(&p.incident_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.incident_id)),
    };

    // Verify incident exists
    match crate::repo::incident_repo::get_incident(&brain.pool, incident_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("Incident", &p.incident_id),
        Err(e) => return error_result(&format!("Database error: {e}")),
    }

    let mut linked = Vec::new();

    // Link servers
    if let Some(slugs) = &p.server_slugs {
        for slug in slugs {
            match crate::repo::server_repo::get_server_by_slug(&brain.pool, slug).await {
                Ok(Some(server)) => {
                    if let Err(e) = crate::repo::incident_repo::link_incident_server(
                        &brain.pool,
                        incident_id,
                        server.id,
                    )
                    .await
                    {
                        return error_result(&format!("Failed to link server '{slug}': {e}"));
                    }
                    linked.push(format!("server:{slug}"));
                }
                Ok(None) => return not_found_with_suggestions(&brain.pool, "Server", slug).await,
                Err(e) => return error_result(&format!("Database error: {e}")),
            }
        }
    }

    // Link services
    if let Some(slugs) = &p.service_slugs {
        for slug in slugs {
            match crate::repo::service_repo::get_service_by_slug(&brain.pool, slug).await {
                Ok(Some(service)) => {
                    if let Err(e) = crate::repo::incident_repo::link_incident_service(
                        &brain.pool,
                        incident_id,
                        service.id,
                    )
                    .await
                    {
                        return error_result(&format!("Failed to link service '{slug}': {e}"));
                    }
                    linked.push(format!("service:{slug}"));
                }
                Ok(None) => return not_found_with_suggestions(&brain.pool, "Service", slug).await,
                Err(e) => return error_result(&format!("Database error: {e}")),
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
                return error_result(&msg);
            }
            match crate::repo::runbook_repo::get_runbook_by_slug(&brain.pool, &rb_link.slug).await {
                Ok(Some(runbook)) => {
                    if let Err(e) = crate::repo::incident_repo::link_incident_runbook(
                        &brain.pool,
                        incident_id,
                        runbook.id,
                        usage,
                    )
                    .await
                    {
                        return error_result(&format!(
                            "Failed to link runbook '{}': {e}",
                            rb_link.slug
                        ));
                    }
                    linked.push(format!("runbook:{}", rb_link.slug));
                }
                Ok(None) => {
                    return not_found_with_suggestions(&brain.pool, "Runbook", &rb_link.slug).await
                }
                Err(e) => return error_result(&format!("Database error: {e}")),
            }
        }
    }

    // Link vendors
    if let Some(names) = &p.vendor_names {
        for name in names {
            match crate::repo::vendor_repo::get_vendor_by_name(&brain.pool, name).await {
                Ok(Some(vendor)) => {
                    if let Err(e) = crate::repo::incident_repo::link_incident_vendor(
                        &brain.pool,
                        incident_id,
                        vendor.id,
                    )
                    .await
                    {
                        return error_result(&format!("Failed to link vendor '{name}': {e}"));
                    }
                    linked.push(format!("vendor:{name}"));
                }
                Ok(None) => return not_found_vendor_with_suggestions(&brain.pool, name).await,
                Err(e) => return error_result(&format!("Database error: {e}")),
            }
        }
    }

    CallToolResult::success(vec![Content::text(format!(
        "Linked to incident {}: {}",
        p.incident_id,
        linked.join(", ")
    ))])
}
