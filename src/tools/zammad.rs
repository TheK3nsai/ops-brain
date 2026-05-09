use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{error_result, json_result, not_found_with_suggestions};
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTicketsParams {
    /// Client slug to filter tickets by Zammad organization (optional — omit for all clients)
    pub client_slug: Option<String>,
    /// Filter by state: "new", "open", "pending_reminder", "closed" (optional, default: all)
    pub state: Option<String>,
    /// Filter by priority: "low", "normal", "high" (optional)
    pub priority: Option<String>,
    /// Maximum number of tickets to return (default: 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetTicketParams {
    /// Zammad ticket ID (integer)
    pub ticket_id: i64,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateTicketParams {
    /// Ticket title
    pub title: String,
    /// Initial message body (becomes the first article)
    pub body: String,
    /// Client slug — resolves to Zammad group, org, and customer
    pub client_slug: String,
    /// Priority: "low", "normal" (default), or "high"
    pub priority: Option<String>,
    /// State: "new" (default), "open", "closed"
    pub state: Option<String>,
    /// Comma-separated tags (e.g. "soporte-usuario,infraestructura")
    pub tags: Option<String>,
    /// Time spent in minutes for time accounting
    pub time_unit: Option<f64>,
    /// Time accounting type: 1=Maintenance, 2=On-site, 3=Remote, 4=On-site/Remote
    pub time_accounting_type_id: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchTicketsParams {
    /// Search query text (Zammad Elasticsearch syntax)
    pub query: String,
    /// Maximum results (default: 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

// ===== HANDLERS =====

pub(crate) async fn handle_list_tickets(
    brain: &super::OpsBrain,
    p: ListTicketsParams,
) -> CallToolResult {
    let zammad = match &brain.zammad_config {
        Some(c) => c,
        None => return error_result("Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)"),
    };

    let mut query_parts = Vec::new();

    // If client_slug provided, scope to that client's Zammad org
    let client_label = if let Some(ref slug) = p.client_slug {
        let client = match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => c,
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        };
        let org_id = match client.zammad_org_id {
            Some(id) => id,
            None => {
                return error_result(&format!(
                    "Client '{slug}' has no Zammad org ID configured. Use upsert_client to set zammad_org_id."
                ))
            }
        };
        query_parts.push(format!("organization.id:{org_id}"));
        slug.clone()
    } else {
        "all".to_string()
    };

    if let Some(ref state) = p.state {
        query_parts.push(format!("state.name:{state}"));
    }
    if let Some(ref priority) = p.priority {
        query_parts.push(format!("priority.name:\"{priority}\""));
    }

    // If no filters at all, use wildcard to get all tickets
    let query = if query_parts.is_empty() {
        "*".to_string()
    } else {
        query_parts.join(" AND ")
    };
    let limit = p.limit.unwrap_or(20);

    match crate::zammad::search_tickets(zammad, &query, limit).await {
        Ok(tickets) => {
            let result = serde_json::json!({
                "count": tickets.len(),
                "client": client_label,
                "tickets": tickets,
            });
            json_result(&result)
        }
        Err(e) => error_result(&e),
    }
}

pub(crate) async fn handle_get_ticket(
    brain: &super::OpsBrain,
    p: GetTicketParams,
) -> CallToolResult {
    let zammad = match &brain.zammad_config {
        Some(c) => c,
        None => return error_result("Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)"),
    };

    let ticket = match crate::zammad::get_ticket(zammad, p.ticket_id).await {
        Ok(t) => t,
        Err(e) => return error_result(&e),
    };

    let articles = match crate::zammad::get_ticket_articles(zammad, p.ticket_id).await {
        Ok(a) => a,
        Err(e) => return error_result(&e),
    };

    let result = serde_json::json!({
        "ticket": ticket,
        "articles": articles,
    });
    json_result(&result)
}

pub(crate) async fn handle_create_ticket(
    brain: &super::OpsBrain,
    p: CreateTicketParams,
) -> CallToolResult {
    let zammad = match &brain.zammad_config {
        Some(c) => c,
        None => return error_result("Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)"),
    };

    let client = match crate::repo::client_repo::get_client_by_slug(&brain.pool, &p.client_slug)
        .await
    {
        Ok(Some(c)) => c,
        Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", &p.client_slug).await,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    let (group_id, customer_id, org_id) = match (client.zammad_group_id, client.zammad_customer_id, client.zammad_org_id) {
        (Some(g), Some(c), org) => (g as i64, c as i64, org.map(|o| o as i64)),
        _ => return error_result(&format!(
            "Client '{}' missing Zammad IDs. Set zammad_group_id and zammad_customer_id via upsert_client.",
            p.client_slug
        )),
    };

    let state_id = match &p.state {
        Some(s) => match crate::zammad::state_name_to_id(s) {
            Some(id) => Some(id),
            None => {
                return error_result(&format!(
                    "Unknown state: '{s}'. Use: new, open, pending_reminder, closed"
                ))
            }
        },
        None => None,
    };

    let priority_id = match &p.priority {
        Some(pr) => match crate::zammad::priority_name_to_id(pr) {
            Some(id) => Some(id),
            None => {
                return error_result(&format!("Unknown priority: '{pr}'. Use: low, normal, high"))
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
        owner_id: zammad.default_owner_id,
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
        Err(e) => return error_result(&e),
    };

    json_result(&ticket)
}

pub(crate) async fn handle_search_tickets(
    brain: &super::OpsBrain,
    p: SearchTicketsParams,
) -> CallToolResult {
    let zammad = match &brain.zammad_config {
        Some(c) => c,
        None => return error_result("Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)"),
    };
    let limit = p.limit.unwrap_or(20);

    match crate::zammad::search_tickets(zammad, &p.query, limit).await {
        Ok(tickets) => {
            let result = serde_json::json!({
                "count": tickets.len(),
                "query": p.query,
                "tickets": tickets,
            });
            json_result(&result)
        }
        Err(e) => error_result(&e),
    }
}
