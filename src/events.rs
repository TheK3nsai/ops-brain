//! Opt-in MCP 2026-07-28 HTTP adapter and durable handoff pointer webhooks.
//! Legacy requests continue through rmcp. This adapter implements only discovery,
//! the existing tools, and webhook events; it advertises no other capabilities.
use crate::{
    auth::{AuthState, CallerClass},
    oauth::{OAuthPrincipal, OAuthState},
    tools::OpsBrain,
};
use axum::{
    body::{to_bytes, Body},
    extract::{Request, State},
    http::{HeaderMap, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::STANDARD, Engine};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::Deserialize;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, PgPool};
use std::{
    net::{IpAddr, SocketAddr},
    sync::Arc,
    time::Duration,
};

const VERSION: &str = "2026-07-28";
const EVENT: &str = "handoff.created";
// A valid 100KB handoff body can occupy 600KB after JSON escaping. Keep
// inbound tool requests separate from the protocol's outbound webhook cap.
const MAX_MCP_REQUEST_BYTES: usize = 650_000;
const MAX_WEBHOOK_BODY_BYTES: usize = 262_144;
const MAX_TTL_MS: i64 = 86_400_000;
const MAX_SUBSCRIPTIONS: i64 = 16;

#[derive(Clone)]
pub struct EventsState {
    pool: PgPool,
    brain: OpsBrain,
    auth: AuthState,
    oauth: Option<Arc<OAuthState>>,
    allowed_hosts: Vec<String>,
    #[cfg(test)]
    test_callbacks: Option<Arc<TestCallbacks>>,
}

#[cfg(test)]
#[derive(Default)]
struct TestCallbacks {
    verifications: std::sync::atomic::AtomicUsize,
    deliveries: std::sync::Mutex<Vec<TestDelivery>>,
}
#[cfg(test)]
struct TestDelivery {
    body: Value,
    signatures: String,
    signed_at: i64,
}
impl EventsState {
    pub fn new(
        pool: PgPool,
        brain: OpsBrain,
        auth: AuthState,
        oauth: Option<Arc<OAuthState>>,
        mut allowed_hosts: Vec<String>,
    ) -> Self {
        if allowed_hosts.is_empty() {
            allowed_hosts = vec!["localhost".into(), "127.0.0.1".into(), "[::1]".into()];
        }
        Self {
            pool,
            brain,
            auth,
            oauth,
            allowed_hosts,
            #[cfg(test)]
            test_callbacks: None,
        }
    }
}

fn digest(value: &[u8]) -> String {
    Sha256::digest(value)
        .iter()
        .map(|v| format!("{v:02x}"))
        .collect()
}
fn static_owner(token: &crate::auth::AgentToken) -> String {
    digest(
        &serde_json::to_vec(&json!([
            "static",
            token.token,
            token.from_agent.to_ascii_lowercase(),
            token.exclude_client_data,
            token.client
        ]))
        .expect("serializable binding"),
    )
}
fn oauth_owner(p: &OAuthPrincipal, token: &crate::auth::AgentToken) -> String {
    digest(
        &serde_json::to_vec(&json!([
            "oauth",
            p.issuer,
            p.subject,
            p.oauth_client_id,
            token.from_agent.to_ascii_lowercase(),
            token.exclude_client_data,
            token.client
        ]))
        .expect("serializable principal binding"),
    )
}
#[derive(Clone)]
struct Owner {
    key: String,
    agent: String,
    restricted: bool,
    oauth: Option<OAuthPrincipal>,
}
impl Owner {
    fn from_request(request: &Request) -> Option<Self> {
        let CallerClass::Agent(token) = request.extensions().get::<CallerClass>()? else {
            return None;
        };
        let oauth = request.extensions().get::<OAuthPrincipal>().cloned();
        let key = oauth
            .as_ref()
            .map_or_else(|| static_owner(token), |p| oauth_owner(p, token));
        Some(Self {
            key,
            agent: token.from_agent.clone(),
            restricted: token.exclude_client_data,
            oauth,
        })
    }
}

#[derive(Debug)]
struct RpcError {
    code: i32,
    message: &'static str,
    data: Option<Value>,
}
impl RpcError {
    fn invalid() -> Self {
        Self {
            code: -32602,
            message: "Invalid event parameters",
            data: None,
        }
    }
    fn forbidden() -> Self {
        Self {
            code: -32024,
            message: "Event access denied",
            data: None,
        }
    }
    fn callback(reason: &'static str) -> Self {
        Self {
            code: -32015,
            message: "CallbackEndpointError",
            data: Some(json!({"reason":reason})),
        }
    }
    fn internal() -> Self {
        Self {
            code: -32603,
            message: "Event operation failed",
            data: None,
        }
    }
}
fn rpc_error(id: Value, e: RpcError, status: StatusCode) -> Response {
    (status, Json(json!({"jsonrpc":"2.0","id":id,"error":{"code":e.code,"message":e.message,"data":e.data}}))).into_response()
}
fn rpc_ok(id: Value, mut result: Value) -> Response {
    result["resultType"] = json!("complete");
    Json(json!({"jsonrpc":"2.0","id":id,"result":result})).into_response()
}
fn header<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get(name)?.to_str().ok()
}
fn host_allowed(request: &Request, allowed: &[String]) -> bool {
    let authority = header(request.headers(), "host")
        .and_then(|s| s.parse::<axum::http::uri::Authority>().ok())
        .or_else(|| request.uri().authority().cloned());
    authority.is_some_and(|actual| {
        allowed.iter().any(|s| {
            let Ok(expected) = s.parse::<axum::http::uri::Authority>() else {
                return false;
            };
            actual
                .host()
                .trim_matches(['[', ']'])
                .eq_ignore_ascii_case(expected.host().trim_matches(['[', ']']))
                && expected
                    .port_u16()
                    .is_none_or(|port| actual.port_u16() == Some(port))
        })
    })
}

/// Install after authentication and only around the `/mcp` service.
pub async fn mcp_events(
    State(state): State<Arc<EventsState>>,
    request: Request,
    next: Next,
) -> Response {
    if !matches!(request.uri().path(), "/mcp" | "/mcp/") {
        return next.run(request).await;
    }
    if request.method() != axum::http::Method::POST {
        return next.run(request).await;
    }
    // Explicit legacy traffic never has its body consumed or limited here.
    let protocol = header(request.headers(), "mcp-protocol-version");
    let method = header(request.headers(), "mcp-method");
    if protocol.is_some_and(|v| v != VERSION)
        || (protocol.is_none()
            && !method.is_some_and(|v| v == "server/discover" || v.starts_with("events/")))
    {
        return next.run(request).await;
    }
    let owner = Owner::from_request(&request);
    let (parts, body) = request.into_parts();
    let bytes = match to_bytes(body, MAX_MCP_REQUEST_BYTES).await {
        Ok(v) => v,
        Err(_) => return StatusCode::PAYLOAD_TOO_LARGE.into_response(),
    };
    let parsed = serde_json::from_slice::<Value>(&bytes);
    let is_modern = header(&parts.headers, "mcp-protocol-version") == Some(VERSION)
        || parsed.as_ref().ok().is_some_and(|v| {
            v["method"] == "server/discover"
                || v["method"]
                    .as_str()
                    .is_some_and(|s| s.starts_with("events/"))
                || v["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"] == VERSION
        });
    if !is_modern {
        return next
            .run(Request::from_parts(parts, Body::from(bytes)))
            .await;
    }
    let request = Request::from_parts(parts, Body::empty());
    if !host_allowed(&request, &state.allowed_hosts) {
        return StatusCode::FORBIDDEN.into_response();
    }
    let rpc = match parsed {
        Ok(v) => v,
        Err(_) => {
            return rpc_error(
                Value::Null,
                RpcError {
                    code: -32700,
                    message: "Parse error",
                    data: None,
                },
                StatusCode::BAD_REQUEST,
            )
        }
    };
    let id = rpc.get("id").cloned().unwrap_or(Value::Null);
    if rpc["jsonrpc"] != "2.0" || !(id.is_string() || id.is_number()) || !rpc["method"].is_string()
    {
        return rpc_error(
            id,
            RpcError {
                code: -32600,
                message: "Invalid Request",
                data: None,
            },
            StatusCode::BAD_REQUEST,
        );
    }
    if !header(request.headers(), "content-type").is_some_and(|v| {
        v.split(';')
            .next()
            .is_some_and(|v| v.trim().eq_ignore_ascii_case("application/json"))
    }) {
        return StatusCode::UNSUPPORTED_MEDIA_TYPE.into_response();
    }
    let method = rpc["method"].as_str().expect("checked");
    let mut params = rpc.get("params").cloned().unwrap_or_else(|| json!({}));
    if let Err(error) = validate_headers(request.headers(), method, &params) {
        return rpc_error(id, error, StatusCode::BAD_REQUEST);
    }
    params
        .as_object_mut()
        .expect("validated object")
        .remove("_meta");
    let result = match method {
        "server/discover" => Ok(
            json!({"supportedVersions":[VERSION],"capabilities":{"tools":{},"events":{}},"instructions":crate::tools::INSTRUCTIONS,"ttlMs":0,"cacheScope":"private","_meta":{"io.modelcontextprotocol/serverInfo":{"name":"ops-brain","version":env!("CARGO_PKG_VERSION")}}}),
        ),
        "tools/list" => {
            if params.get("cursor").is_some_and(|v| !v.is_null()) {
                Err(RpcError::invalid())
            } else {
                Ok(json!({"tools":modern_tools(&state),"ttlMs":0,"cacheScope":"private"}))
            }
        }
        "tools/call" => {
            let Some(name) = params["name"].as_str() else {
                return rpc_error(id, RpcError::invalid(), StatusCode::OK);
            };
            if params.get("task").is_some() {
                return rpc_error(
                    id,
                    RpcError {
                        code: -32026,
                        message: "Task execution is not supported",
                        data: None,
                    },
                    StatusCode::OK,
                );
            }
            let arguments = params
                .get("arguments")
                .cloned()
                .unwrap_or_else(|| json!({}));
            let (parts, _) = request.into_parts();
            let mut extensions = rmcp::model::Extensions::new();
            extensions.insert(parts);
            match state
                .brain
                .call_modern_tool(name, arguments, extensions)
                .await
            {
                Ok(v) => serde_json::to_value(v).map_err(|_| RpcError::internal()),
                Err(e) => {
                    return (
                        StatusCode::OK,
                        Json(json!({"jsonrpc":"2.0","id":id,"error":e})),
                    )
                        .into_response()
                }
            }
        }
        "events/list" => Ok(
            json!({"events":if owner.is_some() { vec![event_definition()] } else { vec![] },"ttlMs":0,"cacheScope":"private"}),
        ),
        "events/subscribe" => match owner {
            Some(owner) => subscribe(&state, owner, params).await,
            None => Err(RpcError::forbidden()),
        },
        "events/unsubscribe" => match owner {
            Some(owner) => unsubscribe(&state, owner, params).await,
            None => Err(RpcError::forbidden()),
        },
        _ => Err(RpcError {
            code: -32601,
            message: "Method not found",
            data: None,
        }),
    };
    match result {
        Ok(result) => rpc_ok(id, result),
        Err(e) => rpc_error(id, e, StatusCode::OK),
    }
}
fn validate_headers(headers: &HeaderMap, method: &str, params: &Value) -> Result<(), RpcError> {
    let mismatch = || RpcError {
        code: -32020,
        message: "MCP header mismatch",
        data: None,
    };
    let meta = &params["_meta"];
    let Some(version) = meta["io.modelcontextprotocol/protocolVersion"].as_str() else {
        return Err(RpcError::invalid());
    };
    if header(headers, "mcp-protocol-version") != Some(version)
        || header(headers, "mcp-method") != Some(method)
    {
        return Err(mismatch());
    }
    if version != VERSION {
        return Err(RpcError {
            code: -32022,
            message: "Unsupported protocol version",
            data: Some(json!({"supported":[VERSION],"requested":version})),
        });
    }
    if !meta["io.modelcontextprotocol/clientCapabilities"].is_object() {
        return Err(RpcError::invalid());
    }
    if let Some(name) = params["name"].as_str() {
        if header(headers, "mcp-name") != Some(name) {
            return Err(mismatch());
        }
    }
    for name in ["mcp-protocol-version", "mcp-method", "mcp-name"] {
        if headers.get_all(name).iter().count() > 1 {
            return Err(mismatch());
        }
    }
    Ok(())
}
fn modern_tools(state: &EventsState) -> Vec<Value> {
    OpsBrain::modern_tool_catalog()
        .into_iter()
        .map(|tool| {
            let mut tool = serde_json::to_value(tool).expect("serializable tool");
            if let Some(oauth) = &state.oauth {
                tool["securitySchemes"] =
                    json!([{"type":"oauth2","scopes":oauth.required_scopes()}]);
                tool["_meta"]["securitySchemes"] = tool["securitySchemes"].clone();
            }
            tool
        })
        .collect()
}
fn event_definition() -> Value {
    json!({"name":EVENT,"description":"A new handoff explicitly addressed to your authenticated agent. Delivers only its ID; retrieve content with get_handoff. Broadcasts and other agents' queues are excluded.","delivery":["webhook"],"inputSchema":{"type":"object","properties":{"agent_name":{"type":"string","description":"Your concrete token-bound agent identity, including a specific member when the credential allows a family. Wildcards are not accepted."}},"required":["agent_name"],"additionalProperties":false},"payloadSchema":{"type":"object","properties":{"handoff_id":{"type":"string","format":"uuid"}},"required":["handoff_id"],"additionalProperties":false}})
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Arguments {
    agent_name: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Delivery {
    mode: String,
    url: String,
    #[serde(default)]
    secret: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
struct SubscriptionParams {
    name: String,
    arguments: Arguments,
    delivery: Delivery,
    #[serde(default)]
    cursor: Option<Value>,
    #[serde(default)]
    ttl_ms: Option<i64>,
}
fn validate_params(owner: &Owner, params: Value) -> Result<SubscriptionParams, RpcError> {
    let mut p: SubscriptionParams =
        serde_json::from_value(params).map_err(|_| RpcError::invalid())?;
    if p.name != EVENT
        || p.delivery.mode != "webhook"
        || p.cursor.is_some()
        || p.ttl_ms.is_some_and(|v| v <= 0)
    {
        return Err(RpcError::invalid());
    }
    p.arguments.agent_name = crate::validation::validate_agent_name(&p.arguments.agent_name)
        .map_err(|_| RpcError::invalid())?
        .to_string();
    let allowed = if owner.oauth.is_some() {
        owner.agent.eq_ignore_ascii_case(&p.arguments.agent_name)
    } else {
        crate::auth::slug_matches(&owner.agent, &p.arguments.agent_name)
    };
    if !allowed {
        return Err(RpcError::forbidden());
    }
    callback_url(&p.delivery.url)?;
    Ok(p)
}
fn subscription_id(owner: &Owner, url: &str) -> String {
    format!(
        "sub_{}",
        digest(
            &serde_json::to_vec(
                &json!([owner.key,url,EVENT,{"agent_name":owner.agent.to_ascii_lowercase()}])
            )
            .expect("serializable identity")
        )
    )
}
#[derive(FromRow)]
struct Subscription {
    id: String,
    owner_key: String,
    oauth_principal: Option<Value>,
    agent_name: String,
    restricted: bool,
    callback_url: String,
    signing_secret: String,
    previous_secret: Option<String>,
    rotation_until: Option<DateTime<Utc>>,
    verified_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    active: bool,
    generation: i64,
}
async fn subscribe(state: &EventsState, owner: Owner, params: Value) -> Result<Value, RpcError> {
    let p = validate_params(&owner, params)?;
    // A family credential selects one concrete queue; the credential owner
    // stays unchanged for quota/revocation and subscription IDs include it.
    let owner = Owner {
        agent: p.arguments.agent_name.clone(),
        ..owner
    };
    let secret = p.delivery.secret.ok_or_else(RpcError::invalid)?;
    decode_secret(&secret)?;
    let id = subscription_id(&owner, &p.delivery.url);
    let existing = sqlx::query_as::<_, Subscription>(
        "SELECT * FROM mcp_event_subscriptions WHERE id=$1 AND owner_key=$2",
    )
    .bind(&id)
    .bind(&owner.key)
    .fetch_optional(&state.pool)
    .await
    .map_err(|_| RpcError::internal())?;
    let now = Utc::now();
    let cached = existing.as_ref().is_some_and(|s| {
        s.active
            && s.expires_at > now
            && s.signing_secret == secret
            && s.verified_at > now - chrono::Duration::minutes(5)
    });
    if !cached {
        verify_for_state(state, &p.delivery.url, &secret, &id).await?;
    }
    let mut expires_at =
        now + chrono::Duration::milliseconds(p.ttl_ms.unwrap_or(MAX_TTL_MS).min(MAX_TTL_MS));
    if let Some(principal) = &owner.oauth {
        let limit = DateTime::from_timestamp(principal.expires_at as i64, 0)
            .ok_or_else(RpcError::forbidden)?;
        expires_at = expires_at.min(limit);
    }
    if expires_at <= Utc::now() {
        return Err(RpcError::forbidden());
    }
    let mut tx = state.pool.begin().await.map_err(|_| RpcError::internal())?;
    // Serialize quota and concurrent refreshes for a principal.
    sqlx::query("SELECT pg_advisory_xact_lock(hashtextextended($1,0))")
        .bind(&owner.key)
        .execute(&mut *tx)
        .await
        .map_err(|_| RpcError::internal())?;
    let count:i64 = sqlx::query_scalar("SELECT count(*) FROM mcp_event_subscriptions WHERE owner_key=$1 AND active AND expires_at > now() AND id<>$2").bind(&owner.key).bind(&id).fetch_one(&mut *tx).await.map_err(|_| RpcError::internal())?;
    if count >= MAX_SUBSCRIPTIONS {
        return Err(RpcError {
            code: -32025,
            message: "Subscription limit reached",
            data: Some(json!({"limit":"subscriptions","max":MAX_SUBSCRIPTIONS})),
        });
    }
    let principal = owner
        .oauth
        .as_ref()
        .map(serde_json::to_value)
        .transpose()
        .map_err(|_| RpcError::internal())?;
    sqlx::query("INSERT INTO mcp_event_subscriptions (id,owner_key,oauth_principal,agent_name,restricted,callback_url,signing_secret,expires_at) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) ON CONFLICT(id) DO UPDATE SET oauth_principal=EXCLUDED.oauth_principal, restricted=EXCLUDED.restricted, previous_secret=CASE WHEN mcp_event_subscriptions.signing_secret<>EXCLUDED.signing_secret THEN mcp_event_subscriptions.signing_secret ELSE mcp_event_subscriptions.previous_secret END,rotation_until=CASE WHEN mcp_event_subscriptions.signing_secret<>EXCLUDED.signing_secret THEN now()+interval '5 minutes' ELSE mcp_event_subscriptions.rotation_until END,signing_secret=EXCLUDED.signing_secret,expires_at=EXCLUDED.expires_at,active=true,verified_at=$9,updated_at=now(),generation=CASE WHEN NOT mcp_event_subscriptions.active OR mcp_event_subscriptions.expires_at<=now() THEN mcp_event_subscriptions.generation+1 ELSE mcp_event_subscriptions.generation END")
        .bind(&id).bind(&owner.key).bind(principal).bind(&owner.agent).bind(owner.restricted).bind(&p.delivery.url).bind(secret).bind(expires_at).bind(if cached { existing.as_ref().expect("cache exists").verified_at } else { Utc::now() }).execute(&mut *tx).await.map_err(|_| RpcError::internal())?;
    tx.commit().await.map_err(|_| RpcError::internal())?;
    Ok(json!({"id":id,"refreshBefore":expires_at.to_rfc3339(),"cursor":null,"truncated":false}))
}
async fn unsubscribe(state: &EventsState, owner: Owner, params: Value) -> Result<Value, RpcError> {
    let p = validate_params(&owner, params)?;
    // A family credential selects one concrete queue; the credential owner
    // stays unchanged for quota/revocation and subscription IDs include it.
    let owner = Owner {
        agent: p.arguments.agent_name.clone(),
        ..owner
    };
    sqlx::query("UPDATE mcp_event_subscriptions SET active=false,updated_at=now(),generation=generation+1 WHERE id=$1 AND owner_key=$2").bind(subscription_id(&owner,&p.delivery.url)).bind(owner.key).execute(&state.pool).await.map_err(|_| RpcError::internal())?;
    Ok(json!({}))
}

fn decode_secret(secret: &str) -> Result<Vec<u8>, RpcError> {
    let key = STANDARD
        .decode(
            secret
                .strip_prefix("whsec_")
                .ok_or_else(RpcError::invalid)?,
        )
        .map_err(|_| RpcError::invalid())?;
    if !(24..=64).contains(&key.len()) {
        return Err(RpcError::invalid());
    }
    Ok(key)
}
fn signature(
    secret: &str,
    event_id: &str,
    timestamp: i64,
    body: &[u8],
) -> Result<String, RpcError> {
    let mut mac =
        Hmac::<Sha256>::new_from_slice(&decode_secret(secret)?).map_err(|_| RpcError::invalid())?;
    mac.update(event_id.as_bytes());
    mac.update(b".");
    mac.update(timestamp.to_string().as_bytes());
    mac.update(b".");
    mac.update(body);
    Ok(format!(
        "v1,{}",
        STANDARD.encode(mac.finalize().into_bytes())
    ))
}
fn callback_url(raw: &str) -> Result<reqwest::Url, RpcError> {
    if raw.len() > 4096 {
        return Err(RpcError::invalid());
    }
    let url = reqwest::Url::parse(raw).map_err(|_| RpcError::invalid())?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return Err(RpcError::invalid());
    }
    Ok(url)
}
fn public_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v) => {
            let n = u32::from(v);
            ![
                (0x00000000, 8),
                (0x0a000000, 8),
                (0x64400000, 10),
                (0x7f000000, 8),
                (0xa9fe0000, 16),
                (0xac100000, 12),
                (0xc0000000, 24),
                (0xc0000200, 24),
                (0xc0586300, 24),
                (0xc0a80000, 16),
                (0xc6120000, 15),
                (0xc6336400, 24),
                (0xcb007100, 24),
                (0xe0000000, 3),
            ]
            .iter()
            .any(|(base, bits)| n >> (32 - bits) == base >> (32 - bits))
        }
        IpAddr::V6(v) => {
            if let Some(v4) = v.to_ipv4_mapped() {
                return public_ip(IpAddr::V4(v4));
            }
            let s = v.segments();
            (s[0] & 0xe000) == 0x2000
                && !(s[0] == 0x2001 && s[1] < 0x0200)
                && !(s[0] == 0x2001 && s[1] == 0x0db8)
                && s[0] != 0x2002
                && (s[0] & 0xfff0) != 0x3ff0
        }
    }
}
async fn pinned_client(url: &reqwest::Url) -> Result<reqwest::Client, RpcError> {
    let host = url
        .host_str()
        .ok_or_else(RpcError::invalid)?
        .trim_matches(['[', ']']);
    let addresses: Vec<SocketAddr> = tokio::time::timeout(
        Duration::from_secs(5),
        tokio::net::lookup_host((host, url.port_or_known_default().unwrap_or(443))),
    )
    .await
    .map_err(|_| RpcError::callback("timeout"))?
    .map_err(|_| RpcError::callback("connection_refused"))?
    .collect();
    if addresses.is_empty() || addresses.iter().any(|a| !public_ip(a.ip())) {
        return Err(RpcError::callback("challenge_failed"));
    }
    reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
        .pool_max_idle_per_host(0)
        .resolve_to_addrs(host, &addresses)
        .build()
        .map_err(|_| RpcError::internal())
}
async fn signed_post(
    url: &str,
    secret: &str,
    previous: Option<&str>,
    subscription: &str,
    event_id: &str,
    body: Vec<u8>,
) -> Result<reqwest::Response, RpcError> {
    if body.len() > MAX_WEBHOOK_BODY_BYTES {
        return Err(RpcError::internal());
    }
    let url = callback_url(url)?;
    let client = pinned_client(&url).await?;
    let timestamp = Utc::now().timestamp();
    let mut signatures = signature(secret, event_id, timestamp, &body)?;
    if let Some(previous) = previous {
        signatures.push(' ');
        signatures.push_str(&signature(previous, event_id, timestamp, &body)?);
    }
    client
        .post(url)
        .header("Content-Type", "application/json")
        .header("webhook-id", event_id)
        .header("webhook-timestamp", timestamp.to_string())
        .header("webhook-signature", signatures)
        .header("X-MCP-Subscription-Id", subscription)
        .body(body)
        .send()
        .await
        .map_err(|e| {
            RpcError::callback(if e.is_timeout() {
                "timeout"
            } else {
                "connection_refused"
            })
        })
}
async fn verify_for_state(
    state: &EventsState,
    url: &str,
    secret: &str,
    id: &str,
) -> Result<(), RpcError> {
    #[cfg(test)]
    if let Some(callbacks) = &state.test_callbacks {
        callbacks
            .verifications
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        return Ok(());
    }
    // The fixture exists only in unit-test builds. Production always reaches
    // signed HTTPS callback verification with public-address pinning.
    let _ = state;
    verify_callback(url, secret, id).await
}

async fn delivery_status(
    state: &EventsState,
    s: &Subscription,
    previous: Option<&str>,
    event_id: &str,
    body: Vec<u8>,
) -> Result<u16, RpcError> {
    #[cfg(test)]
    if let Some(callbacks) = &state.test_callbacks {
        let timestamp = Utc::now().timestamp();
        let mut signatures = signature(&s.signing_secret, event_id, timestamp, &body)?;
        if let Some(secret) = previous {
            signatures.push(' ');
            signatures.push_str(&signature(secret, event_id, timestamp, &body)?);
        }
        callbacks
            .deliveries
            .lock()
            .expect("fixture mutex")
            .push(TestDelivery {
                body: serde_json::from_slice(&body).expect("fixture JSON event"),
                signatures,
                signed_at: timestamp,
            });
        return Ok(200);
    }
    let _ = state;
    signed_post(
        &s.callback_url,
        &s.signing_secret,
        previous,
        &s.id,
        event_id,
        body,
    )
    .await
    .map(|response| response.status().as_u16())
}

async fn verify_callback(url: &str, secret: &str, id: &str) -> Result<(), RpcError> {
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|_| RpcError::internal())?;
    let challenge = STANDARD.encode(random);
    let event_id = format!("verification_{}", uuid::Uuid::now_v7());
    let body = serde_json::to_vec(&json!({"type":"verification","challenge":challenge}))
        .map_err(|_| RpcError::internal())?;
    let response = signed_post(url, secret, None, id, &event_id, body).await?;
    if !response.status().is_success() {
        return Err(RpcError::callback(if response.status().is_server_error() {
            "http_5xx"
        } else {
            "http_4xx"
        }));
    }
    let mut response = response;
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| RpcError::callback("challenge_failed"))?
    {
        if bytes.len() + chunk.len() > 4096 {
            return Err(RpcError::callback("challenge_failed"));
        }
        bytes.extend_from_slice(&chunk);
    }
    let echoed: Value =
        serde_json::from_slice(&bytes).map_err(|_| RpcError::callback("challenge_failed"))?;
    if !echoed["challenge"]
        .as_str()
        .is_some_and(|s| crate::auth::validate_token(s, &challenge))
    {
        return Err(RpcError::callback("challenge_failed"));
    }
    Ok(())
}

#[derive(FromRow)]
struct QueuedEvent {
    subscription_id: String,
    handoff_id: uuid::Uuid,
    generation: i64,
    occurred_at: DateTime<Utc>,
    attempts: i32,
}
fn subscription_authorized(state: &EventsState, s: &Subscription) -> bool {
    if crate::validation::validate_agent_name(&s.agent_name).is_err() {
        return false;
    }
    if let Some(raw) = &s.oauth_principal {
        let Ok(principal) = serde_json::from_value::<OAuthPrincipal>(raw.clone()) else {
            return false;
        };
        state
            .oauth
            .as_ref()
            .and_then(|oauth| oauth.principal_agent(&principal))
            .is_some_and(|token| {
                token.from_agent.eq_ignore_ascii_case(&s.agent_name)
                    && token.exclude_client_data == s.restricted
                    && oauth_owner(&principal, &token) == s.owner_key
            })
    } else {
        state.auth.agent_tokens.iter().any(|token| {
            static_owner(token) == s.owner_key
                && crate::auth::slug_matches(&token.from_agent, &s.agent_name)
                && token.exclude_client_data == s.restricted
        })
    }
}
fn retry_delay(attempt: i32) -> i64 {
    5 * (1i64 << attempt.clamp(0, 9))
}
fn retryable(status: u16) -> bool {
    status == 408 || status == 429 || (500..600).contains(&status)
}
/// Single worker; SKIP LOCKED also permits several instances without duplicate claims.
/// A crash after HTTP receipt can repeat delivery, preserving the stable event ID.
pub async fn run_worker(state: Arc<EventsState>) {
    let mut interval = tokio::time::interval(Duration::from_secs(2));
    let mut next_maintenance = tokio::time::Instant::now();
    loop {
        interval.tick().await;
        for _ in 0..32 {
            if tokio::time::Instant::now() >= next_maintenance {
                if maintain_events(&state.pool).await.is_err() {
                    tracing::warn!("MCP event maintenance failed");
                }
                next_maintenance = tokio::time::Instant::now() + Duration::from_secs(300);
            }
            match deliver_next(&state).await {
                Ok(true) => {}
                Ok(false) => break,
                Err(e) => {
                    tracing::warn!(error=%e,"MCP event worker database failure");
                    break;
                }
            }
        }
    }
}

async fn maintain_events(pool: &PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE mcp_event_subscriptions SET previous_secret=NULL,rotation_until=NULL WHERE rotation_until<=now()")
        .execute(pool).await?;
    sqlx::query("DELETE FROM mcp_event_outbox WHERE finished_at < now()-interval '1 day'")
        .execute(pool)
        .await?;
    sqlx::query("DELETE FROM mcp_event_subscriptions WHERE expires_at < now()-interval '7 days'")
        .execute(pool)
        .await?;
    Ok(())
}
async fn deliver_next(state: &EventsState) -> Result<bool, sqlx::Error> {
    let mut tx = state.pool.begin().await?;
    let Some(event)=sqlx::query_as::<_,QueuedEvent>("SELECT * FROM mcp_event_outbox WHERE finished_at IS NULL AND next_attempt_at<=now() ORDER BY next_attempt_at LIMIT 1 FOR UPDATE SKIP LOCKED").fetch_optional(&mut *tx).await? else{return Ok(false);};
    // Lock remains held through bounded HTTPS send: unsubscribe waits for in-flight
    // delivery and guarantees no send begins after its successful response.
    let subscription = sqlx::query_as::<_, Subscription>(
        "SELECT * FROM mcp_event_subscriptions WHERE id=$1 FOR UPDATE",
    )
    .bind(&event.subscription_id)
    .fetch_optional(&mut *tx)
    .await?;
    let mut retry = false;
    if let Some(s) = subscription {
        let visible: bool = sqlx::query_scalar(
            "SELECT EXISTS(SELECT 1 FROM handoffs WHERE id=$1 AND lower(to_agent)=lower($2))",
        )
        .bind(event.handoff_id)
        .bind(&s.agent_name)
        .fetch_one(&mut *tx)
        .await?;
        if s.active
            && s.expires_at > Utc::now()
            && s.generation == event.generation
            && visible
            && subscription_authorized(state, &s)
        {
            let event_id = format!("evt_{}", event.handoff_id);
            let body=serde_json::to_vec(&json!({"eventId":event_id,"name":EVENT,"timestamp":event.occurred_at.to_rfc3339(),"data":{"handoff_id":event.handoff_id},"cursor":null})).expect("serializable pointer");
            let previous = s
                .previous_secret
                .as_deref()
                .filter(|_| s.rotation_until.is_some_and(|until| until > Utc::now()));
            let deadline = (s.expires_at - Utc::now())
                .to_std()
                .unwrap_or(Duration::ZERO)
                .min(Duration::from_secs(15));
            match tokio::time::timeout(
                deadline,
                delivery_status(state, &s, previous, &event_id, body),
            )
            .await
            {
                Ok(result) => match result {
                    Ok(response) => {
                        retry = retryable(response);
                        if response == 410 {
                            sqlx::query("UPDATE mcp_event_subscriptions SET active=false,updated_at=now() WHERE id=$1").bind(&s.id).execute(&mut *tx).await?;
                        }
                    }
                    Err(_) => retry = true,
                },
                Err(_) => retry = true,
            }
        }
    }
    if retry && event.attempts < 7 {
        sqlx::query("UPDATE mcp_event_outbox SET attempts=attempts+1,next_attempt_at=now()+make_interval(secs=>$3::double precision) WHERE subscription_id=$1 AND handoff_id=$2").bind(&event.subscription_id).bind(event.handoff_id).bind(retry_delay(event.attempts) as f64).execute(&mut *tx).await?;
    } else {
        sqlx::query("UPDATE mcp_event_outbox SET attempts=attempts+1,finished_at=now() WHERE subscription_id=$1 AND handoff_id=$2").bind(&event.subscription_id).bind(event.handoff_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    // The fixture enables real SQL/trigger/lifecycle coverage without requiring
    // a public callback receiver. CI supplies its disposable PostgreSQL URL.
    #[tokio::test]
    #[ignore = "requires OPS_BRAIN_EVENTS_TEST_DATABASE_URL pointing at disposable PostgreSQL"]
    async fn durable_subscription_lifecycle() {
        use std::sync::atomic::Ordering;
        let url = std::env::var("OPS_BRAIN_EVENTS_TEST_DATABASE_URL")
            .expect("explicit disposable test database URL");
        let pool = sqlx::postgres::PgPoolOptions::new()
            .max_connections(5)
            .connect(&url)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let token = crate::auth::AgentToken {
            token: format!("fixture-{}", uuid::Uuid::now_v7()),
            from_agent: format!("Fixture-{}", uuid::Uuid::now_v7()),
            client: None,
            exclude_client_data: true,
        };
        let owner = Owner {
            key: static_owner(&token),
            agent: token.from_agent.clone(),
            restricted: true,
            oauth: None,
        };
        let auth = AuthState {
            main_token: Some("fixture-main".into()),
            agent_tokens: Arc::new(vec![token]),
            machine_tokens: Arc::new(vec![]),
        };
        let callbacks = Arc::new(TestCallbacks::default());
        let new_state = || {
            let mut state = EventsState::new(
                pool.clone(),
                OpsBrain::new(pool.clone(), None),
                auth.clone(),
                None,
                vec![],
            );
            state.test_callbacks = Some(callbacks.clone());
            state
        };
        let state = new_state();
        let secret_a = format!("whsec_{}", STANDARD.encode([1u8; 32]));
        let secret_b = format!("whsec_{}", STANDARD.encode([2u8; 32]));
        let callback = "https://receiver.example/hook";
        let params = |callback: &str, secret: &str| json!({"name":EVENT,"arguments":{"agent_name":owner.agent},"delivery":{"mode":"webhook","url":callback,"secret":secret},"ttlMs":60000});
        let result = subscribe(&state, owner.clone(), params(callback, &secret_a))
            .await
            .unwrap();
        let id = result["id"].as_str().unwrap().to_string();
        assert_eq!(id, subscription_id(&owner, callback));
        let row =
            sqlx::query_as::<_, Subscription>("SELECT * FROM mcp_event_subscriptions WHERE id=$1")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(row.active);
        assert_eq!(row.generation, 1);
        assert!(row.expires_at <= Utc::now() + chrono::Duration::seconds(60));
        assert!(row.expires_at > Utc::now() + chrono::Duration::seconds(50));
        assert_eq!(callbacks.verifications.load(Ordering::Relaxed), 1);
        let refreshed = subscribe(&state, owner.clone(), params(callback, &secret_a))
            .await
            .unwrap();
        assert_eq!(refreshed["id"], result["id"]);
        assert_eq!(
            callbacks.verifications.load(Ordering::Relaxed),
            1,
            "bounded verification cache"
        );
        let rotated = subscribe(&state, owner.clone(), params(callback, &secret_b))
            .await
            .unwrap();
        assert_eq!(rotated["id"], result["id"]);
        assert_eq!(callbacks.verifications.load(Ordering::Relaxed), 2);
        let row =
            sqlx::query_as::<_, Subscription>("SELECT * FROM mcp_event_subscriptions WHERE id=$1")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row.signing_secret, secret_b);
        assert_eq!(row.previous_secret.as_deref(), Some(secret_a.as_str()));
        assert!(row.rotation_until.unwrap() > Utc::now());
        let mut handoff_ids = Vec::new();
        async fn enqueue(pool: &PgPool, agent: &str) -> uuid::Uuid {
            let id = uuid::Uuid::now_v7();
            sqlx::query("INSERT INTO handoffs(id,from_agent,to_agent,title,body) VALUES($1,'Fixture-Sender',$2,'fixture','private content must never appear in event')").bind(id).bind(agent).execute(pool).await.unwrap();
            id
        }
        let first = enqueue(&pool, &owner.agent).await;
        handoff_ids.push(first);
        drop(state);
        // Construct a new state, with no subscription cache, before reading the
        // durable outbox and signing with the keys recovered from PostgreSQL.
        let restarted = new_state();
        assert!(deliver_next(&restarted).await.unwrap());
        {
            let delivered = callbacks.deliveries.lock().unwrap();
            assert_eq!(delivered.len(), 1);
            let event = &delivered[0];
            assert_eq!(event.body["data"], json!({"handoff_id":first}));
            assert_eq!(event.body["name"], EVENT);
            assert_eq!(event.body["eventId"], format!("evt_{first}"));
            assert!(!event.body.to_string().contains("private content"));
            let bytes = serde_json::to_vec(&event.body).unwrap();
            let expected = format!(
                "{} {}",
                signature(&secret_b, &format!("evt_{first}"), event.signed_at, &bytes).unwrap(),
                signature(&secret_a, &format!("evt_{first}"), event.signed_at, &bytes).unwrap()
            );
            assert_eq!(
                event.signatures, expected,
                "new and old keys sign the exact pointer bytes during rotation"
            );
        }
        let finished:bool=sqlx::query_scalar("SELECT finished_at IS NOT NULL FROM mcp_event_outbox WHERE subscription_id=$1 AND handoff_id=$2").bind(&id).bind(first).fetch_one(&pool).await.unwrap();
        assert!(finished);
        sqlx::query("UPDATE mcp_event_subscriptions SET rotation_until=now()-interval '1 second' WHERE id=$1")
            .bind(&id).execute(&pool).await.unwrap();
        maintain_events(&pool).await.unwrap();
        let maintained =
            sqlx::query_as::<_, Subscription>("SELECT * FROM mcp_event_subscriptions WHERE id=$1")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            maintained.previous_secret.is_none(),
            "expired signing keys are removed from active grants"
        );
        assert!(maintained.rotation_until.is_none());
        assert_eq!(maintained.signing_secret, secret_b);
        let stale = enqueue(&pool, &owner.agent).await;
        handoff_ids.push(stale);
        let mut unsubscribe_params = params(callback, &secret_b);
        unsubscribe_params["delivery"]
            .as_object_mut()
            .unwrap()
            .remove("secret");
        unsubscribe_params.as_object_mut().unwrap().remove("ttlMs");
        assert_eq!(
            unsubscribe(&restarted, owner.clone(), unsubscribe_params.clone())
                .await
                .unwrap(),
            json!({})
        );
        assert_eq!(
            unsubscribe(&restarted, owner.clone(), unsubscribe_params)
                .await
                .unwrap(),
            json!({})
        );
        let row =
            sqlx::query_as::<_, Subscription>("SELECT * FROM mcp_event_subscriptions WHERE id=$1")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!row.active);
        assert!(row.generation > 1);
        let resumed = subscribe(&new_state(), owner.clone(), params(callback, &secret_b))
            .await
            .unwrap();
        assert_eq!(resumed["id"], result["id"]);
        let row =
            sqlx::query_as::<_, Subscription>("SELECT * FROM mcp_event_subscriptions WHERE id=$1")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(row.active);
        assert!(row.generation > 1);
        assert!(deliver_next(&new_state()).await.unwrap());
        assert_eq!(
            callbacks.deliveries.lock().unwrap().len(),
            1,
            "old generation cannot deliver after resubscribe/restart"
        );
        let expired = enqueue(&pool, &owner.agent).await;
        handoff_ids.push(expired);
        sqlx::query(
            "UPDATE mcp_event_subscriptions SET expires_at=now()-interval '1 second' WHERE id=$1",
        )
        .bind(&id)
        .execute(&pool)
        .await
        .unwrap();
        assert!(deliver_next(&new_state()).await.unwrap());
        assert_eq!(
            callbacks.deliveries.lock().unwrap().len(),
            1,
            "expired subscriptions stop queued sends"
        );
        subscribe(&new_state(), owner.clone(), params(callback, &secret_b))
            .await
            .unwrap();
        let current = enqueue(&pool, &owner.agent).await;
        handoff_ids.push(current);
        assert!(deliver_next(&new_state()).await.unwrap());
        assert_eq!(
            callbacks.deliveries.lock().unwrap().len(),
            2,
            "fresh generation resumes delivery after expiration/re-subscribe"
        );
        for n in 1..MAX_SUBSCRIPTIONS {
            subscribe(
                &restarted,
                owner.clone(),
                params(&format!("https://receiver.example/hook/{n}"), &secret_b),
            )
            .await
            .unwrap();
        }
        let over_quota = subscribe(
            &restarted,
            owner.clone(),
            params("https://receiver.example/overflow", &secret_b),
        )
        .await
        .unwrap_err();
        assert_eq!(over_quota.code, -32025);
        assert!(
            subscribe(&restarted, owner.clone(), params(callback, &secret_b))
                .await
                .is_ok(),
            "refresh does not consume another quota slot"
        );
        let count: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM mcp_event_subscriptions WHERE owner_key=$1 AND active",
        )
        .bind(&owner.key)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, MAX_SUBSCRIPTIONS);
        sqlx::query("DELETE FROM mcp_event_subscriptions WHERE owner_key=$1")
            .bind(&owner.key)
            .execute(&pool)
            .await
            .unwrap();
        // One family credential may own several concrete subscriptions, with
        // independent IDs and unsubscribe lifecycle even at the same callback.
        let family_token = crate::auth::AgentToken {
            token: format!("fixture-family-{}", uuid::Uuid::now_v7()),
            from_agent: format!("Family-{}-*", uuid::Uuid::now_v7()),
            client: None,
            exclude_client_data: true,
        };
        let family_owner = Owner {
            key: static_owner(&family_token),
            agent: family_token.from_agent.clone(),
            restricted: true,
            oauth: None,
        };
        let family_base = family_owner.agent.strip_suffix("-*").unwrap();
        let first_member = format!("{family_base}-Coder");
        let second_member = format!("{family_base}-Researcher");
        let mut family_state = EventsState::new(
            pool.clone(),
            OpsBrain::new(pool.clone(), None),
            AuthState {
                main_token: Some("fixture-main".into()),
                agent_tokens: Arc::new(vec![family_token]),
                machine_tokens: Arc::new(vec![]),
            },
            None,
            vec![],
        );
        family_state.test_callbacks = Some(callbacks.clone());
        let mut first_params = params(callback, &secret_b);
        first_params["arguments"]["agent_name"] = json!(first_member);
        let mut second_params = params(callback, &secret_b);
        second_params["arguments"]["agent_name"] = json!(second_member);
        let first_subscription =
            subscribe(&family_state, family_owner.clone(), first_params.clone())
                .await
                .unwrap();
        let second_subscription = subscribe(&family_state, family_owner.clone(), second_params)
            .await
            .unwrap();
        assert_ne!(first_subscription["id"], second_subscription["id"]);
        let queues: Vec<String> = sqlx::query_scalar(
            "SELECT agent_name FROM mcp_event_subscriptions WHERE owner_key=$1 ORDER BY agent_name",
        )
        .bind(&family_owner.key)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(queues, vec![first_member, second_member]);
        unsubscribe(&family_state, family_owner.clone(), first_params)
            .await
            .unwrap();
        let second_active: bool =
            sqlx::query_scalar("SELECT active FROM mcp_event_subscriptions WHERE id=$1")
                .bind(second_subscription["id"].as_str().unwrap())
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(second_active);
        sqlx::query("DELETE FROM mcp_event_subscriptions WHERE owner_key=$1")
            .bind(&family_owner.key)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM handoffs WHERE id=ANY($1)")
            .bind(&handoff_ids)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn oauth_remapping_policy_changes_and_expiry_invalidate_old_grants() {
        let principal = OAuthPrincipal {
            issuer: "https://id.example.com".into(),
            subject: "fixture-user".into(),
            oauth_client_id: "fixture-client".into(),
            expires_at: (Utc::now() + chrono::Duration::hours(1)).timestamp() as u64,
            scopes: vec![],
        };
        let token = crate::auth::AgentToken {
            token: String::new(),
            from_agent: "Test-Bot".into(),
            exclude_client_data: true,
            client: Some("example".into()),
        };
        let mut subscription = Subscription {
            id: "sub_fixture".into(),
            owner_key: oauth_owner(&principal, &token),
            oauth_principal: Some(serde_json::to_value(&principal).unwrap()),
            agent_name: token.from_agent.clone(),
            restricted: true,
            callback_url: "https://example.com/hook".into(),
            signing_secret: String::new(),
            previous_secret: None,
            rotation_until: None,
            verified_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            active: true,
            generation: 1,
        };
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://fixture@127.0.0.1/unused")
            .unwrap();
        for (agent, restricted, client, expected) in [
            ("Test-Bot", true, "example", true),
            ("Other-Bot", true, "example", false),
            ("Test-Bot", false, "example", false),
            ("Test-Bot", true, "other", false),
        ] {
            let config = json!({"issuer":principal.issuer,"jwks_uri":"https://id.example.com/keys","bindings":[{"subject":principal.subject,"oauth_client_id":principal.oauth_client_id,"from_agent":agent,"exclude_client_data":restricted,"client":client}]});
            let oauth = Arc::new(
                OAuthState::from_json(&config.to_string(), "https://bus.example.com/mcp").unwrap(),
            );
            let state = EventsState::new(
                pool.clone(),
                OpsBrain::new(pool.clone(), None),
                AuthState {
                    main_token: Some("fixture".into()),
                    agent_tokens: Arc::new(vec![]),
                    machine_tokens: Arc::new(vec![]),
                },
                Some(oauth),
                vec![],
            );
            assert_eq!(subscription_authorized(&state, &subscription), expected);
            let mut expired = serde_json::to_value(&principal).unwrap();
            expired["expires_at"] = json!(1);
            subscription.oauth_principal = Some(expired);
            assert!(!subscription_authorized(&state, &subscription));
            subscription.oauth_principal = Some(serde_json::to_value(&principal).unwrap());
        }
    }
    #[test]
    fn private_and_special_networks_are_blocked() {
        for ip in [
            "127.0.0.1",
            "10.0.0.1",
            "100.64.0.1",
            "169.254.169.254",
            "172.16.0.1",
            "192.168.0.1",
            "192.0.2.1",
            "198.18.0.1",
            "198.51.100.1",
            "203.0.113.1",
            "224.0.0.1",
            "255.255.255.255",
            "::1",
            "::",
            "fc00::1",
            "fe80::1",
            "::ffff:127.0.0.1",
            "2001:db8::1",
            "2002:7f00:1::",
        ] {
            assert!(!public_ip(ip.parse().unwrap()), "{ip}");
        }
        for ip in ["8.8.8.8", "1.1.1.1", "2606:4700:4700::1111"] {
            assert!(public_ip(ip.parse().unwrap()), "{ip}");
        }
    }
    #[test]
    fn callback_is_https_without_credentials_or_fragment() {
        for url in [
            "http://example.com",
            "https://user:pass@example.com",
            "https://example.com/#fragment",
            "file:///tmp/test",
        ] {
            assert!(callback_url(url).is_err());
        }
        assert!(callback_url("https://example.com/callback?token=opaque").is_ok());
    }
    #[test]
    fn secret_bounds_and_signature_are_standard_webhooks() {
        let secret = format!("whsec_{}", STANDARD.encode([0u8; 32]));
        assert!(decode_secret(&secret).is_ok());
        assert!(decode_secret(&format!("whsec_{}", STANDARD.encode([0u8; 23]))).is_err());
        assert!(decode_secret(&format!("whsec_{}", STANDARD.encode([0u8; 65]))).is_err());
        assert!(decode_secret("whsec_invalid").is_err());
        let expected = "v1,DCwoDUtLWBqvVcB8S4INgK2EFh4Geqzed1wKjGgPklI=";
        assert_eq!(
            signature(
                &secret,
                "evt_fixture",
                1700000000,
                b"{\"data\":{\"handoff_id\":\"fixture\"}}"
            )
            .unwrap(),
            expected
        );
    }
    #[test]
    fn only_authenticated_inbound_queue_is_subscribable() {
        let owner = Owner {
            key: "opaque".into(),
            agent: "Test-Bot".into(),
            restricted: true,
            oauth: None,
        };
        let params = json!({"name":EVENT,"arguments":{"agent_name":"test-bot"},"delivery":{"mode":"webhook","url":"https://example.com/hook"}});
        assert!(validate_params(&owner, params.clone()).is_ok());
        let mut other = params.clone();
        other["arguments"]["agent_name"] = json!("Other-Bot");
        assert_eq!(validate_params(&owner, other).err().unwrap().code, -32024);
        let mut arbitrary = params.clone();
        arbitrary["arguments"]["anything"] = json!(true);
        assert!(validate_params(&owner, arbitrary).is_err());
        let mut replay = params;
        replay["cursor"] = json!("old");
        assert!(validate_params(&owner, replay).is_err());
        assert_eq!(
            subscription_id(&owner, "https://example.com/hook"),
            subscription_id(&owner, "https://example.com/hook")
        );
        assert_ne!(
            subscription_id(&owner, "https://example.com/hook"),
            subscription_id(&owner, "https://example.com/other")
        );
    }
    #[tokio::test]
    async fn family_credentials_select_concrete_queues_and_worker_rechecks_binding() {
        let token = crate::auth::AgentToken {
            token: "fixture-family-credential".into(),
            from_agent: "Test-Bot-*".into(),
            client: None,
            exclude_client_data: true,
        };
        let owner = Owner {
            key: static_owner(&token),
            agent: token.from_agent.clone(),
            restricted: true,
            oauth: None,
        };
        let params = |agent: &str| json!({"name":EVENT,"arguments":{"agent_name":agent},"delivery":{"mode":"webhook","url":"https://example.com/hook"}});
        for agent in ["Test-Bot", "Test-Bot-Coder", "test-bot-Researcher"] {
            assert!(validate_params(&owner, params(agent)).is_ok());
        }
        for agent in ["Another-Bot", "Test-Botnet", "Test-Bot-"] {
            assert_eq!(
                validate_params(&owner, params(agent)).err().unwrap().code,
                -32024
            );
        }
        assert_eq!(
            validate_params(&owner, params("Test-Bot-*"))
                .err()
                .unwrap()
                .code,
            -32602
        );
        let coder = Owner {
            agent: "Test-Bot-Coder".into(),
            ..owner.clone()
        };
        let researcher = Owner {
            agent: "Test-Bot-Researcher".into(),
            ..owner.clone()
        };
        assert_ne!(
            subscription_id(&coder, "https://example.com/hook"),
            subscription_id(&researcher, "https://example.com/hook")
        );
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgres://fixture@127.0.0.1/unused")
            .unwrap();
        let state = EventsState::new(
            pool.clone(),
            OpsBrain::new(pool, None),
            AuthState {
                main_token: Some("fixture".into()),
                agent_tokens: Arc::new(vec![token]),
                machine_tokens: Arc::new(vec![]),
            },
            None,
            vec![],
        );
        let mut subscription = Subscription {
            id: "sub_fixture".into(),
            owner_key: owner.key,
            oauth_principal: None,
            agent_name: coder.agent,
            restricted: true,
            callback_url: "https://example.com/hook".into(),
            signing_secret: String::new(),
            previous_secret: None,
            rotation_until: None,
            verified_at: Utc::now(),
            expires_at: Utc::now() + chrono::Duration::hours(1),
            active: true,
            generation: 1,
        };
        assert!(subscription_authorized(&state, &subscription));
        subscription.agent_name = "Another-Bot".into();
        assert!(!subscription_authorized(&state, &subscription));
        subscription.agent_name = "Test-Bot-*".into();
        assert!(!subscription_authorized(&state, &subscription));
        subscription.agent_name = "Test-Bot-Coder".into();
        subscription.restricted = false;
        assert!(!subscription_authorized(&state, &subscription));
        subscription.restricted = true;
        subscription.owner_key = "different-credential".into();
        assert!(!subscription_authorized(&state, &subscription));
    }
    #[test]
    fn modern_headers_cannot_disagree_or_omit_metadata() {
        let mut headers = HeaderMap::new();
        headers.insert("mcp-protocol-version", VERSION.parse().unwrap());
        headers.insert("mcp-method", "tools/call".parse().unwrap());
        headers.insert("mcp-name", "check_in".parse().unwrap());
        let params = json!({"name":"check_in","_meta":{"io.modelcontextprotocol/protocolVersion":VERSION,"io.modelcontextprotocol/clientCapabilities":{}}});
        assert!(validate_headers(&headers, "tools/call", &params).is_ok());
        headers.insert("mcp-name", "other".parse().unwrap());
        assert_eq!(
            validate_headers(&headers, "tools/call", &params)
                .err()
                .unwrap()
                .code,
            -32020
        );
        assert!(validate_headers(&headers, "tools/call", &json!({})).is_err());
    }
    #[test]
    fn retry_is_bounded_to_transient_responses() {
        for code in [408, 429, 500, 503] {
            assert!(retryable(code));
        }
        for code in [200, 301, 400, 401, 403, 404, 410, 413] {
            assert!(!retryable(code));
        }
        assert_eq!(retry_delay(0), 5);
        assert_eq!(retry_delay(7), 640);
    }
}
