//! Local protocol and database fixtures; no public receiver or live vendor needed.
mod common;

use axum::{
    body::{to_bytes, Body},
    http::{Request, StatusCode},
    middleware,
    routing::post,
    Router,
};
use ops_brain::{
    auth::{self, AgentToken, AuthState},
    events::{self, EventsState},
    tools::OpsBrain,
};
use serde_json::{json, Value};
use sqlx::PgPool;
use std::sync::Arc;
use tower::ServiceExt;
use uuid::Uuid;

const TOKEN: &str = "fixture-agent-credential-0123456789abcdef";
fn auth_state(agent: &str) -> AuthState {
    AuthState {
        main_token: Some("fixture-main".into()),
        machine_tokens: Arc::new(vec![]),
        agent_tokens: Arc::new(vec![AgentToken {
            token: TOKEN.into(),
            from_agent: agent.into(),
            client: None,
            exclude_client_data: true,
        }]),
    }
}
fn app(pool: PgPool, agent: &str) -> Router {
    let auth = auth_state(agent);
    let state = Arc::new(EventsState::new(
        pool.clone(),
        OpsBrain::new(pool, None),
        auth.clone(),
        None,
        vec!["localhost".into()],
    ));
    Router::new()
        .route("/mcp", post(|| async { "legacy" }))
        .route("/api/handoff", post(|| async { "REST" }))
        .layer(middleware::from_fn_with_state(state, events::mcp_events))
        .layer(middleware::from_fn_with_state(auth, auth::bearer_auth))
}
fn request(method: &str, mut params: Value) -> Request<Body> {
    params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}});
    let mut request = Request::post("/mcp")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("host", "localhost")
        .header("content-type", "application/json")
        .header("mcp-protocol-version", "2026-07-28")
        .header("mcp-method", method);
    if let Some(name) = params["name"].as_str() {
        request = request.header("mcp-name", name);
    }
    request
        .body(Body::from(
            json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}).to_string(),
        ))
        .unwrap()
}
async fn response_json(response: axum::response::Response) -> Value {
    serde_json::from_slice(&to_bytes(response.into_body(), 262144).await.unwrap()).unwrap()
}

#[tokio::test]
async fn modern_discovery_catalog_and_tool_identity_reuse_existing_guards() {
    let pool = common::test_pool().await;
    let agent = format!("Test-{}", Uuid::now_v7());
    let app = app(pool, &agent);
    let legacy = Request::post("/mcp")
        .header("authorization", format!("Bearer {TOKEN}"))
        .header("host", "localhost")
        .header("mcp-protocol-version", "2025-11-25")
        .body(Body::from(vec![b'x'; 300_000]))
        .unwrap();
    let legacy = app.clone().oneshot(legacy).await.unwrap();
    assert_eq!(legacy.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(legacy.into_body(), 100).await.unwrap()[..],
        b"legacy"
    );

    let discovery = response_json(
        app.clone()
            .oneshot(request("server/discover", json!({})))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(discovery["result"]["resultType"], "complete");
    assert_eq!(discovery["result"]["supportedVersions"][0], "2026-07-28");
    let catalog = response_json(
        app.clone()
            .oneshot(request("tools/list", json!({})))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(catalog["result"]["tools"].as_array().unwrap().len(), 13);
    assert_eq!(catalog["result"]["cacheScope"], "private");
    let events = response_json(
        app.clone()
            .oneshot(request("events/list", json!({})))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(events["result"]["events"][0]["name"], "handoff.created");
    let mismatch=response_json(app.clone().oneshot(request("tools/call",json!({"name":"create_handoff","arguments":{"from_agent":"Another-Agent","to_agent":agent,"title":"fixture","body":"fixture"}}))).await.unwrap()).await;
    assert_eq!(mismatch["result"]["isError"], true);
    assert!(mismatch.to_string().contains("token-bound"));
    // JSON escaping expands valid stored text up to sixfold. The modern
    // transport must still reach the tool's usual validation/identity guard.
    let escaped = app
        .clone()
        .oneshot(request(
            "tools/call",
            json!({
                "name":"create_handoff", "arguments":{
                    "from_agent":"Another-Agent", "to_agent":agent, "title":"fixture",
                    "body":"\u{0001}".repeat(ops_brain::validation::MAX_BODY_BYTES)
                }
            }),
        ))
        .await
        .unwrap();
    assert_eq!(escaped.status(), StatusCode::OK);
    assert!(response_json(escaped)
        .await
        .to_string()
        .contains("token-bound"));

    let other_queue=response_json(app.clone().oneshot(request("events/subscribe",json!({"name":"handoff.created","arguments":{"agent_name":"Another-Agent"},"delivery":{"mode":"webhook","url":"https://example.com/hook","secret":"whsec_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}}))).await.unwrap()).await;
    assert_eq!(other_queue["error"]["code"], -32024);
    let mut host = request("server/discover", json!({}));
    host.headers_mut()
        .insert("host", "attacker.invalid".parse().unwrap());
    assert_eq!(
        app.clone().oneshot(host).await.unwrap().status(),
        StatusCode::FORBIDDEN
    );
    let mut mismatch = request("tools/list", json!({}));
    mismatch
        .headers_mut()
        .insert("mcp-method", "tools/call".parse().unwrap());
    let mismatch = response_json(app.clone().oneshot(mismatch).await.unwrap()).await;
    assert_eq!(mismatch["error"]["code"], -32020);
    let private=response_json(app.oneshot(request("events/subscribe",json!({"name":"handoff.created","arguments":{"agent_name":agent},"delivery":{"mode":"webhook","url":"https://127.0.0.1/hook","secret":"whsec_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}}))).await.unwrap()).await;
    assert_eq!(private["error"]["code"], -32015);
    assert_eq!(private["error"]["data"]["reason"], "challenge_failed");
}

#[tokio::test]
async fn durable_outbox_includes_explicit_recipient_only_and_revocation_discards_delivery() {
    let pool = common::test_pool().await;
    let agent = format!("Test-{}", Uuid::now_v7());
    let sub = format!("sub_fixture_{}", Uuid::now_v7());
    sqlx::query("INSERT INTO mcp_event_subscriptions(id,owner_key,agent_name,restricted,callback_url,signing_secret,expires_at) VALUES($1,'revoked-credential',$2,true,'https://127.0.0.1/never-connect','whsec_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=',now()+interval '1 hour')").bind(&sub).bind(&agent).execute(&pool).await.unwrap();
    let mut ids = vec![];
    for recipient in [Some(agent.as_str()), Some("Another-Agent"), None] {
        let id = Uuid::now_v7();
        ids.push(id);
        sqlx::query("INSERT INTO handoffs(id,from_agent,to_agent,title,body) VALUES($1,'Fixture-Sender',$2,'fixture','fixture')").bind(id).bind(recipient).execute(&pool).await.unwrap();
    }
    let queued: Vec<Uuid> =
        sqlx::query_scalar("SELECT handoff_id FROM mcp_event_outbox WHERE subscription_id=$1")
            .bind(&sub)
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(queued, vec![ids[0]]);
    // Worker reconstructs durable state and rejects the no-longer-configured
    // credential before opening even a local callback connection.
    let state = Arc::new(EventsState::new(
        pool.clone(),
        OpsBrain::new(pool.clone(), None),
        auth_state(&agent),
        None,
        vec![],
    ));
    let worker = tokio::spawn(events::run_worker(state));
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let finished: bool = sqlx::query_scalar(
                "SELECT finished_at IS NOT NULL FROM mcp_event_outbox WHERE subscription_id=$1",
            )
            .bind(&sub)
            .fetch_one(&pool)
            .await
            .unwrap();
            if finished {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    worker.abort();
    let attempts: i32 =
        sqlx::query_scalar("SELECT attempts FROM mcp_event_outbox WHERE subscription_id=$1")
            .bind(&sub)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(attempts, 1);
    // Expired grants never enqueue fresh work, and signing keys survive restarts
    // only until bounded expiry/retention cleanup.
    sqlx::query(
        "UPDATE mcp_event_subscriptions SET expires_at=now()-interval '1 second' WHERE id=$1",
    )
    .bind(&sub)
    .execute(&pool)
    .await
    .unwrap();
    let expired = Uuid::now_v7();
    ids.push(expired);
    sqlx::query("INSERT INTO handoffs(id,from_agent,to_agent,title,body) VALUES($1,'Fixture-Sender',$2,'fixture','fixture')").bind(expired).bind(&agent).execute(&pool).await.unwrap();
    let count: i64 =
        sqlx::query_scalar("SELECT count(*) FROM mcp_event_outbox WHERE subscription_id=$1")
            .bind(&sub)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(count, 1);
    sqlx::query("DELETE FROM mcp_event_subscriptions WHERE id=$1")
        .bind(&sub)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("DELETE FROM handoffs WHERE id=ANY($1)")
        .bind(&ids)
        .execute(&pool)
        .await
        .unwrap();
}

#[tokio::test]
async fn family_event_subscriptions_require_one_authorized_concrete_member() {
    let pool = common::test_pool().await;
    let app = app(pool, "Test-Bot-*");
    let params = |agent: &str| json!({"name":"handoff.created","arguments":{"agent_name":agent},"delivery":{"mode":"webhook","url":"https://127.0.0.1/hook","secret":"whsec_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA="}});
    let own = response_json(
        app.clone()
            .oneshot(request("events/subscribe", params("Test-Bot-Coder")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(
        own["error"]["code"], -32015,
        "authorized concrete queue advances to callback verification"
    );
    let foreign = response_json(
        app.clone()
            .oneshot(request("events/subscribe", params("Other-Bot")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(foreign["error"]["code"], -32024);
    let wildcard = response_json(
        app.oneshot(request("events/subscribe", params("Test-Bot-*")))
            .await
            .unwrap(),
    )
    .await;
    assert_eq!(wildcard["error"]["code"], -32602);
}
