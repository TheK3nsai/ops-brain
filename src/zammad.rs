use serde::{Deserialize, Deserializer, Serialize};

/// Deserialize a value that may be a number or a string representation of a number.
/// Zammad inconsistently returns `time_unit` as either `1.5` (number) or `"1.5"` (string).
fn deserialize_string_or_f64<'de, D>(deserializer: D) -> Result<Option<f64>, D::Error>
where
    D: Deserializer<'de>,
{
    use serde::de;

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum StringOrNumber {
        Number(f64),
        String(String),
        Null,
    }

    match Option::<StringOrNumber>::deserialize(deserializer)? {
        Some(StringOrNumber::Number(n)) => Ok(Some(n)),
        Some(StringOrNumber::String(s)) => {
            if s.is_empty() {
                Ok(None)
            } else {
                s.parse::<f64>().map(Some).map_err(de::Error::custom)
            }
        }
        Some(StringOrNumber::Null) | None => Ok(None),
    }
}

/// Configuration for connecting to Zammad REST API
#[derive(Debug, Clone)]
pub struct ZammadConfig {
    pub base_url: String,
    pub api_token: String,
    pub default_owner_id: Option<i64>,
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
    #[serde(default, deserialize_with = "deserialize_string_or_f64")]
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
    #[serde(default, deserialize_with = "deserialize_string_or_f64")]
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
///
/// Zammad returns different response shapes depending on the query:
/// - Filtered queries (e.g. `organization.id:2`): direct JSON array of tickets
/// - Unfiltered/wildcard queries (e.g. `*`): JSON object with `assets.Ticket` map
///
/// This function handles both cases transparently.
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

    let body: serde_json::Value = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse Zammad search response: {e}"))?;

    parse_ticket_search_response(body)
}

/// Parse Zammad ticket search response, handling both array and object envelope formats.
fn parse_ticket_search_response(body: serde_json::Value) -> Result<Vec<ZammadTicket>, String> {
    // Case 1: Direct array of expanded tickets
    if body.is_array() {
        return serde_json::from_value::<Vec<ZammadTicket>>(body)
            .map_err(|e| format!("Failed to parse Zammad ticket array: {e}"));
    }

    // Case 2: Object envelope — extract from assets.Ticket map
    if let Some(assets) = body.get("assets").and_then(|a| a.get("Ticket")) {
        if let Some(map) = assets.as_object() {
            let mut tickets: Vec<ZammadTicket> = Vec::with_capacity(map.len());
            for (_id, val) in map {
                match serde_json::from_value::<ZammadTicket>(val.clone()) {
                    Ok(t) => tickets.push(t),
                    Err(e) => {
                        tracing::warn!("Skipping unparseable ticket in assets: {e}");
                    }
                }
            }
            // Sort by ID descending (newest first) since HashMap has no order
            tickets.sort_by_key(|t| std::cmp::Reverse(t.id));
            return Ok(tickets);
        }
    }

    // Case 3: Object with no assets.Ticket — might be empty result or unknown shape
    // Check if it has a "tickets" array (some Zammad versions)
    if let Some(arr) = body.get("tickets") {
        if arr.is_array() {
            // This contains ticket IDs, not full objects — return empty with a note
            // (the expand=true param should prevent this, but handle gracefully)
            return Ok(Vec::new());
        }
    }

    // Fallback: empty result
    Ok(Vec::new())
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
            default_owner_id: None,
        };
        assert_eq!(auth_header(&config), "Token token=abc123");
    }

    // api_url

    #[test]
    fn api_url_construction() {
        let config = ZammadConfig {
            base_url: "http://localhost:3000".to_string(),
            api_token: "test".to_string(),
            default_owner_id: None,
        };
        assert_eq!(
            api_url(&config, "/tickets"),
            "http://localhost:3000/api/v1/tickets"
        );
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
            default_owner_id: None,
        };
        assert_eq!(
            api_url(&config, "/tickets"),
            "http://localhost:3000/api/v1/tickets"
        );
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
            "group": "Support",
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

    // parse_ticket_search_response

    #[test]
    fn deserialize_time_unit_as_string() {
        let json = r#"{
            "id": 10, "number": "10010", "title": "Time unit string",
            "time_unit": "1.5",
            "created_at": "2026-03-27T10:00:00Z",
            "updated_at": "2026-03-27T10:00:00Z"
        }"#;
        let ticket: ZammadTicket = serde_json::from_str(json).unwrap();
        assert_eq!(ticket.time_unit, Some(1.5));
    }

    #[test]
    fn deserialize_time_unit_as_number() {
        let json = r#"{
            "id": 11, "number": "10011", "title": "Time unit number",
            "time_unit": 2.0,
            "created_at": "2026-03-27T10:00:00Z",
            "updated_at": "2026-03-27T10:00:00Z"
        }"#;
        let ticket: ZammadTicket = serde_json::from_str(json).unwrap();
        assert_eq!(ticket.time_unit, Some(2.0));
    }

    #[test]
    fn deserialize_time_unit_null() {
        let json = r#"{
            "id": 12, "number": "10012", "title": "Time unit null",
            "time_unit": null,
            "created_at": "2026-03-27T10:00:00Z",
            "updated_at": "2026-03-27T10:00:00Z"
        }"#;
        let ticket: ZammadTicket = serde_json::from_str(json).unwrap();
        assert_eq!(ticket.time_unit, None);
    }

    #[test]
    fn parse_response_array_with_string_time_unit() {
        let body = serde_json::json!([
            {
                "id": 1, "number": "00001", "title": "Ticket A",
                "state": "open", "time_unit": "1.5",
                "created_at": "2026-03-20T10:00:00Z",
                "updated_at": "2026-03-20T10:00:00Z"
            }
        ]);
        let tickets = parse_ticket_search_response(body).unwrap();
        assert_eq!(tickets.len(), 1);
        assert_eq!(tickets[0].time_unit, Some(1.5));
    }

    #[test]
    fn parse_response_array_format() {
        let body = serde_json::json!([
            {
                "id": 1, "number": "00001", "title": "Ticket A",
                "state": "open", "created_at": "2026-03-20T10:00:00Z",
                "updated_at": "2026-03-20T10:00:00Z"
            },
            {
                "id": 2, "number": "00002", "title": "Ticket B",
                "state": "closed", "created_at": "2026-03-20T11:00:00Z",
                "updated_at": "2026-03-20T11:00:00Z"
            }
        ]);
        let tickets = parse_ticket_search_response(body).unwrap();
        assert_eq!(tickets.len(), 2);
        assert_eq!(tickets[0].title, "Ticket A");
        assert_eq!(tickets[1].title, "Ticket B");
    }

    #[test]
    fn parse_response_assets_envelope() {
        // Zammad returns this shape for unfiltered/wildcard queries
        let body = serde_json::json!({
            "tickets": [2, 1],
            "tickets_count": 2,
            "assets": {
                "Ticket": {
                    "1": {
                        "id": 1, "number": "00001", "title": "Ticket A",
                        "created_at": "2026-03-20T10:00:00Z",
                        "updated_at": "2026-03-20T10:00:00Z"
                    },
                    "2": {
                        "id": 2, "number": "00002", "title": "Ticket B",
                        "created_at": "2026-03-20T11:00:00Z",
                        "updated_at": "2026-03-20T11:00:00Z"
                    }
                }
            }
        });
        let tickets = parse_ticket_search_response(body).unwrap();
        assert_eq!(tickets.len(), 2);
        // Sorted by ID descending (newest first)
        assert_eq!(tickets[0].id, 2);
        assert_eq!(tickets[1].id, 1);
    }

    #[test]
    fn parse_response_empty_object() {
        let body = serde_json::json!({});
        let tickets = parse_ticket_search_response(body).unwrap();
        assert!(tickets.is_empty());
    }

    #[test]
    fn parse_response_tickets_ids_only() {
        // Edge case: object with tickets array but no assets
        let body = serde_json::json!({
            "tickets": [1, 2],
            "tickets_count": 2
        });
        let tickets = parse_ticket_search_response(body).unwrap();
        assert!(tickets.is_empty()); // graceful fallback
    }
}
