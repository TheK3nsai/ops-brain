use serde::{Deserialize, Serialize};

/// Configuration for connecting to Zammad REST API
#[derive(Debug, Clone)]
pub struct ZammadConfig {
    pub base_url: String,
    pub api_token: String,
}

// ===== State/Priority name↔ID mappings =====

pub fn state_name_to_id(name: &str) -> Option<i64> {
    match name.to_lowercase().as_str() {
        "new" => Some(1),
        "open" => Some(2),
        "pending reminder" | "pending_reminder" => Some(3),
        "closed" => Some(4),
        _ => None,
    }
}

pub fn priority_name_to_id(name: &str) -> Option<i64> {
    match name.to_lowercase().as_str() {
        "low" | "1 low" => Some(1),
        "normal" | "2 normal" => Some(2),
        "high" | "3 high" => Some(3),
        _ => None,
    }
}

// ===== Response structs (from Zammad with ?expand=true) =====

/// Zammad ticket as returned by the API with ?expand=true
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZammadTicket {
    pub id: i64,
    pub number: String,
    pub title: String,
    #[serde(default)]
    pub group: Option<String>,
    #[serde(default)]
    pub group_id: Option<i64>,
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub state_id: Option<i64>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub priority_id: Option<i64>,
    #[serde(default)]
    pub owner: Option<String>,
    #[serde(default)]
    pub owner_id: Option<i64>,
    #[serde(default)]
    pub customer: Option<String>,
    #[serde(default)]
    pub customer_id: Option<i64>,
    #[serde(default)]
    pub organization: Option<String>,
    #[serde(default)]
    pub organization_id: Option<i64>,
    #[serde(default)]
    pub tags: Option<String>,
    #[serde(default)]
    pub time_unit: Option<f64>,
    #[serde(default)]
    pub article_count: Option<i64>,
    pub created_at: String,
    pub updated_at: String,
}

/// Zammad ticket article (message/note on a ticket)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ZammadArticle {
    pub id: i64,
    pub ticket_id: i64,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default)]
    pub body: Option<String>,
    #[serde(default)]
    pub content_type: Option<String>,
    #[serde(rename = "type")]
    #[serde(default)]
    pub article_type: Option<String>,
    #[serde(default)]
    pub internal: Option<bool>,
    #[serde(default)]
    pub time_unit: Option<f64>,
    pub created_at: String,
    pub updated_at: String,
}

// ===== Payload structs (for creating/updating) =====

#[derive(Debug, Serialize)]
pub struct CreateTicketPayload {
    pub title: String,
    pub group_id: i64,
    pub customer_id: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub organization_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tags: Option<String>,
    pub article: CreateArticleInline,
}

#[derive(Debug, Serialize)]
pub struct CreateArticleInline {
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub article_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_unit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_accounting_type_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct UpdateTicketPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_id: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct CreateArticlePayload {
    pub ticket_id: i64,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
    #[serde(rename = "type")]
    #[serde(skip_serializing_if = "Option::is_none")]
    pub article_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub internal: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_unit: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub time_accounting_type_id: Option<i64>,
}

// ===== API functions =====

fn build_client(_config: &ZammadConfig) -> reqwest::Client {
    reqwest::Client::new()
}

fn auth_header(config: &ZammadConfig) -> String {
    format!("Token token={}", config.api_token)
}

fn api_url(config: &ZammadConfig, path: &str) -> String {
    format!("{}/api/v1{}", config.base_url.trim_end_matches('/'), path)
}

/// Search Zammad tickets. Uses Elasticsearch query syntax.
pub async fn search_tickets(
    config: &ZammadConfig,
    query: &str,
    limit: i64,
) -> Result<Vec<ZammadTicket>, String> {
    let client = build_client(config);
    let url = api_url(config, "/tickets/search");

    let response = client
        .get(&url)
        .header("Authorization", auth_header(config))
        .query(&[
            ("query", query),
            ("expand", "true"),
            ("limit", &limit.to_string()),
        ])
        .send()
        .await
        .map_err(|e| format!("Failed to search Zammad tickets: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Zammad search returned HTTP {status}: {body}"));
    }

    response
        .json::<Vec<ZammadTicket>>()
        .await
        .map_err(|e| format!("Failed to parse Zammad search response: {e}"))
}

/// Get a single ticket by ID
pub async fn get_ticket(config: &ZammadConfig, ticket_id: i64) -> Result<ZammadTicket, String> {
    let client = build_client(config);
    let url = api_url(config, &format!("/tickets/{ticket_id}"));

    let response = client
        .get(&url)
        .header("Authorization", auth_header(config))
        .query(&[("expand", "true")])
        .send()
        .await
        .map_err(|e| format!("Failed to fetch Zammad ticket {ticket_id}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Zammad GET ticket {ticket_id} returned HTTP {status}: {body}"
        ));
    }

    response
        .json::<ZammadTicket>()
        .await
        .map_err(|e| format!("Failed to parse Zammad ticket response: {e}"))
}

/// Get all articles for a ticket
pub async fn get_ticket_articles(
    config: &ZammadConfig,
    ticket_id: i64,
) -> Result<Vec<ZammadArticle>, String> {
    let client = build_client(config);
    let url = api_url(config, &format!("/ticket_articles/by_ticket/{ticket_id}"));

    let response = client
        .get(&url)
        .header("Authorization", auth_header(config))
        .send()
        .await
        .map_err(|e| format!("Failed to fetch articles for ticket {ticket_id}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Zammad GET articles for ticket {ticket_id} returned HTTP {status}: {body}"
        ));
    }

    response
        .json::<Vec<ZammadArticle>>()
        .await
        .map_err(|e| format!("Failed to parse Zammad articles response: {e}"))
}

/// Create a new ticket in Zammad
pub async fn create_ticket(
    config: &ZammadConfig,
    payload: &CreateTicketPayload,
) -> Result<ZammadTicket, String> {
    let client = build_client(config);
    let url = api_url(config, "/tickets");

    let response = client
        .post(&url)
        .header("Authorization", auth_header(config))
        .header("Content-Type", "application/json")
        .query(&[("expand", "true")])
        .json(payload)
        .send()
        .await
        .map_err(|e| format!("Failed to create Zammad ticket: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!("Zammad POST ticket returned HTTP {status}: {body}"));
    }

    response
        .json::<ZammadTicket>()
        .await
        .map_err(|e| format!("Failed to parse Zammad create ticket response: {e}"))
}

/// Update an existing ticket
pub async fn update_ticket(
    config: &ZammadConfig,
    ticket_id: i64,
    payload: &UpdateTicketPayload,
) -> Result<ZammadTicket, String> {
    let client = build_client(config);
    let url = api_url(config, &format!("/tickets/{ticket_id}"));

    let response = client
        .put(&url)
        .header("Authorization", auth_header(config))
        .header("Content-Type", "application/json")
        .query(&[("expand", "true")])
        .json(payload)
        .send()
        .await
        .map_err(|e| format!("Failed to update Zammad ticket {ticket_id}: {e}"))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Zammad PUT ticket {ticket_id} returned HTTP {status}: {body}"
        ));
    }

    response
        .json::<ZammadTicket>()
        .await
        .map_err(|e| format!("Failed to parse Zammad update ticket response: {e}"))
}

/// Add an article (note/reply) to a ticket
pub async fn add_ticket_article(
    config: &ZammadConfig,
    payload: &CreateArticlePayload,
) -> Result<ZammadArticle, String> {
    let client = build_client(config);
    let url = api_url(config, "/ticket_articles");

    let response = client
        .post(&url)
        .header("Authorization", auth_header(config))
        .header("Content-Type", "application/json")
        .json(payload)
        .send()
        .await
        .map_err(|e| format!("Failed to add article to ticket {}: {e}", payload.ticket_id))?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await.unwrap_or_default();
        return Err(format!(
            "Zammad POST article for ticket {} returned HTTP {status}: {body}",
            payload.ticket_id
        ));
    }

    response
        .json::<ZammadArticle>()
        .await
        .map_err(|e| format!("Failed to parse Zammad article response: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // state_name_to_id

    #[test]
    fn state_mappings() {
        assert_eq!(state_name_to_id("new"), Some(1));
        assert_eq!(state_name_to_id("open"), Some(2));
        assert_eq!(state_name_to_id("pending reminder"), Some(3));
        assert_eq!(state_name_to_id("pending_reminder"), Some(3));
        assert_eq!(state_name_to_id("closed"), Some(4));
    }

    #[test]
    fn state_case_insensitive() {
        assert_eq!(state_name_to_id("NEW"), Some(1));
        assert_eq!(state_name_to_id("Open"), Some(2));
        assert_eq!(state_name_to_id("CLOSED"), Some(4));
    }

    #[test]
    fn state_invalid() {
        assert_eq!(state_name_to_id("deleted"), None);
        assert_eq!(state_name_to_id(""), None);
        assert_eq!(state_name_to_id("pending"), None);
    }

    // priority_name_to_id

    #[test]
    fn priority_mappings() {
        assert_eq!(priority_name_to_id("low"), Some(1));
        assert_eq!(priority_name_to_id("normal"), Some(2));
        assert_eq!(priority_name_to_id("high"), Some(3));
    }

    #[test]
    fn priority_expanded_names() {
        assert_eq!(priority_name_to_id("1 low"), Some(1));
        assert_eq!(priority_name_to_id("2 normal"), Some(2));
        assert_eq!(priority_name_to_id("3 high"), Some(3));
    }

    #[test]
    fn priority_case_insensitive() {
        assert_eq!(priority_name_to_id("LOW"), Some(1));
        assert_eq!(priority_name_to_id("Normal"), Some(2));
        assert_eq!(priority_name_to_id("HIGH"), Some(3));
    }

    #[test]
    fn priority_invalid() {
        assert_eq!(priority_name_to_id("critical"), None);
        assert_eq!(priority_name_to_id("urgent"), None);
        assert_eq!(priority_name_to_id(""), None);
    }

    // auth_header

    #[test]
    fn auth_header_format() {
        let config = ZammadConfig {
            base_url: "http://localhost:3000".to_string(),
            api_token: "abc123".to_string(),
        };
        assert_eq!(auth_header(&config), "Token token=abc123");
    }

    // api_url

    #[test]
    fn api_url_construction() {
        let config = ZammadConfig {
            base_url: "http://localhost:3000".to_string(),
            api_token: "test".to_string(),
        };
        assert_eq!(api_url(&config, "/tickets"), "http://localhost:3000/api/v1/tickets");
        assert_eq!(
            api_url(&config, "/tickets/123"),
            "http://localhost:3000/api/v1/tickets/123"
        );
    }

    #[test]
    fn api_url_trims_trailing_slash() {
        let config = ZammadConfig {
            base_url: "http://localhost:3000/".to_string(),
            api_token: "test".to_string(),
        };
        assert_eq!(api_url(&config, "/tickets"), "http://localhost:3000/api/v1/tickets");
    }

    // Deserialization

    #[test]
    fn deserialize_zammad_ticket() {
        let json = r#"{
            "id": 42,
            "number": "12345",
            "title": "Test ticket",
            "state": "open",
            "state_id": 2,
            "priority": "normal",
            "priority_id": 2,
            "group": "HSR",
            "group_id": 2,
            "created_at": "2026-03-20T10:00:00Z",
            "updated_at": "2026-03-20T11:00:00Z"
        }"#;

        let ticket: ZammadTicket = serde_json::from_str(json).unwrap();
        assert_eq!(ticket.id, 42);
        assert_eq!(ticket.number, "12345");
        assert_eq!(ticket.title, "Test ticket");
        assert_eq!(ticket.state, Some("open".to_string()));
        assert_eq!(ticket.state_id, Some(2));
    }

    #[test]
    fn deserialize_zammad_ticket_minimal() {
        // Minimal fields — all optional fields default to None
        let json = r#"{
            "id": 1,
            "number": "00001",
            "title": "Minimal",
            "created_at": "2026-03-20T10:00:00Z",
            "updated_at": "2026-03-20T10:00:00Z"
        }"#;

        let ticket: ZammadTicket = serde_json::from_str(json).unwrap();
        assert_eq!(ticket.id, 1);
        assert!(ticket.state.is_none());
        assert!(ticket.priority.is_none());
        assert!(ticket.tags.is_none());
        assert!(ticket.time_unit.is_none());
    }

    #[test]
    fn serialize_create_ticket_payload() {
        let payload = CreateTicketPayload {
            title: "Test".to_string(),
            group_id: 2,
            customer_id: 5,
            organization_id: Some(2),
            state_id: Some(1),
            priority_id: Some(2),
            owner_id: None,
            tags: Some("infraestructura".to_string()),
            article: CreateArticleInline {
                body: "Test body".to_string(),
                content_type: Some("text/plain".to_string()),
                article_type: Some("note".to_string()),
                internal: Some(true),
                time_unit: None,
                time_accounting_type_id: None,
            },
        };

        let json = serde_json::to_value(&payload).unwrap();
        assert_eq!(json["title"], "Test");
        assert_eq!(json["group_id"], 2);
        // owner_id should be skipped (None)
        assert!(json.get("owner_id").is_none());
        assert_eq!(json["article"]["body"], "Test body");
        assert_eq!(json["article"]["internal"], true);
        // time_unit should be skipped
        assert!(json["article"].get("time_unit").is_none());
    }
}
