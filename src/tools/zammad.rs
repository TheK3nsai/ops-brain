use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListTicketsParams {
    /// Client slug to filter tickets by Zammad organization
    pub client_slug: String,
    /// Filter by state: "new", "open", "pending_reminder", "closed" (optional, default: all)
    pub state: Option<String>,
    /// Filter by priority: "low", "normal", "high" (optional)
    pub priority: Option<String>,
    /// Maximum number of tickets to return (default: 20)
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
    /// Optionally link to an ops-brain incident by ID (UUID string)
    pub incident_id: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateTicketParams {
    /// Zammad ticket ID
    pub ticket_id: i64,
    /// New title (optional)
    pub title: Option<String>,
    /// New state: "new", "open", "pending_reminder", "closed" (optional)
    pub state: Option<String>,
    /// New priority: "low", "normal", "high" (optional)
    pub priority: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddTicketNoteParams {
    /// Zammad ticket ID
    pub ticket_id: i64,
    /// Note body text
    pub body: String,
    /// Whether this is an internal note (default: true) vs public reply
    pub internal: Option<bool>,
    /// Time spent in minutes for time accounting on this article
    pub time_unit: Option<f64>,
    /// Time accounting type: 1=Maintenance, 2=On-site, 3=Remote, 4=On-site/Remote
    pub time_accounting_type_id: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchTicketsParams {
    /// Search query text (Zammad Elasticsearch syntax)
    pub query: String,
    /// Maximum results (default: 20)
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkTicketParams {
    /// Zammad ticket ID to link
    pub zammad_ticket_id: i64,
    /// Incident ID to link to (UUID string, optional)
    pub incident_id: Option<String>,
    /// Server slug to link to (optional)
    pub server_slug: Option<String>,
    /// Service slug to link to (optional)
    pub service_slug: Option<String>,
    /// Notes about this link (optional)
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkTicketParams {
    /// Zammad ticket ID to unlink
    pub zammad_ticket_id: i64,
}
