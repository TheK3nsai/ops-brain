use clap::Parser;
use ops_brain::{api, auth, config::Config, db, embeddings, tools::OpsBrain};
use rmcp::service::ServiceExt;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    dotenvy::dotenv().ok();

    tracing_subscriber::fmt()
        .with_writer(std::io::stderr)
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("ops_brain=info")),
        )
        .init();

    let config = Config::parse();

    tracing::info!("Connecting to database...");
    let pool = db::create_pool(&config.database_url).await?;

    if config.migrate {
        tracing::info!("Running migrations...");
        db::run_migrations(&pool).await?;
    }

    let embedding_client = if config.embeddings_enabled.unwrap_or(true) {
        tracing::info!(
            "Embeddings configured: url={}, model={}",
            config.embedding_url,
            config.embedding_model
        );
        Some(embeddings::EmbeddingClient::new(
            config.embedding_url.clone(),
            config.embedding_model.clone(),
            config.embedding_api_key.clone(),
        ))
    } else {
        tracing::info!("Embeddings disabled via OPS_BRAIN_EMBEDDINGS_ENABLED=false");
        None
    };

    let server = OpsBrain::new(pool.clone(), embedding_client.clone());

    match config.transport.as_str() {
        "stdio" => {
            tracing::info!("Starting ops-brain MCP server (stdio transport)");
            let transport = rmcp::transport::io::stdio();
            let service = server.serve(transport).await?;
            service.waiting().await?;
        }
        "http" => {
            use rmcp::transport::streamable_http_server::{
                session::local::LocalSessionManager,
                tower::{StreamableHttpServerConfig, StreamableHttpService},
            };
            use std::sync::Arc;
            use std::time::Duration;

            // rmcp's SessionConfig::DEFAULT_KEEP_ALIVE is 300s — sessions
            // get evicted server-side after 5 minutes of idle, and existing
            // MCP clients (Claude Code's rmcp HTTP client, Gemini CLI's Node
            // SDK) don't auto-reinitialize on the resulting 404. Bumping
            // to 1h covers normal coding pauses while still reaping genuine
            // zombies in reasonable time.
            let mut session_manager = LocalSessionManager::default();
            session_manager.session_config.keep_alive = Some(Duration::from_secs(3600));
            let session_manager = Arc::new(session_manager);

            // Fail closed on auth config. clap maps a present-but-empty env
            // var (`OPS_BRAIN_AUTH_TOKEN=`) to Some(""), and an empty
            // "secret" must never become a valid bearer credential — so
            // blank and missing are both treated as "no token", and no
            // token aborts unless the operator explicitly opted into an
            // open dev server.
            let main_token: Option<String> = config
                .auth_token
                .as_deref()
                .map(str::trim)
                .filter(|t| !t.is_empty())
                .map(str::to_string);
            if main_token.is_none() {
                if config.dev_no_auth {
                    tracing::warn!(
                        "AUTH DISABLED (--dev-no-auth): every caller gets full access. \
                         Never expose this listener beyond localhost."
                    );
                } else {
                    anyhow::bail!(
                        "OPS_BRAIN_AUTH_TOKEN is missing or blank; refusing to serve HTTP \
                         without auth. Set a real token, or pass --dev-no-auth / \
                         OPS_BRAIN_DEV_NO_AUTH=true for a deliberately open dev server."
                    );
                }
            }

            let api_state = Arc::new(api::ApiState { pool: pool.clone() });

            let mut http_config = StreamableHttpServerConfig::default();
            let parsed_hosts: Vec<String> = config
                .allowed_hosts
                .as_deref()
                .map(|h| {
                    h.split(',')
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty())
                        .collect()
                })
                .unwrap_or_default();
            if !parsed_hosts.is_empty() {
                tracing::info!("HTTP allowed_hosts: {:?}", parsed_hosts);
                http_config = http_config.with_allowed_hosts(parsed_hosts);
            } else if config.allowed_hosts.is_some() {
                tracing::warn!(
                    "OPS_BRAIN_ALLOWED_HOSTS set but empty/whitespace; using loopback default. \
                     Empty allowlist disables DNS-rebind protection in rmcp — refusing."
                );
            } else {
                tracing::info!(
                    "HTTP allowed_hosts: loopback default (set OPS_BRAIN_ALLOWED_HOSTS for public deploy)"
                );
            }

            let embedding_client_http = embedding_client.clone();
            let mcp_service = StreamableHttpService::new(
                move || Ok(OpsBrain::new(pool.clone(), embedding_client_http.clone())),
                session_manager,
                http_config,
            );

            // Machine tokens: scoped credentials for the REST ingestion path
            // (POST /api/handoff) and wake poll (GET /api/pending). Parse
            // failures abort startup — a silently dropped token would read
            // as "monitor wired up" while its filings 401.
            let machine_tokens =
                auth::parse_machine_tokens(config.machine_tokens.as_deref(), main_token.as_deref())
                    .map_err(|e| anyhow::anyhow!("OPS_BRAIN_MACHINE_TOKENS: {e}"))?;
            if !machine_tokens.is_empty() {
                tracing::info!(
                    count = machine_tokens.len(),
                    bindings = ?machine_tokens
                        .iter()
                        .map(|t| format!(
                            "{} (client={}, scopes={})",
                            t.from_agent,
                            t.client.as_deref().unwrap_or("-"),
                            t.scopes.join("+")
                        ))
                        .collect::<Vec<_>>(),
                    "machine tokens configured"
                );
            }
            // Per-agent tokens: identity-bound credentials for interactive MCP
            // sessions. Cross-checked against the main + machine secrets so a
            // shared secret can't blur the credential classes. Parse failures
            // abort startup — a dropped agent token would read as "identity
            // enforced" while that agent still files unbound.
            let agent_tokens = auth::parse_agent_tokens(
                config.agent_tokens.as_deref(),
                main_token.as_deref(),
                &machine_tokens,
            )
            .map_err(|e| anyhow::anyhow!("OPS_BRAIN_AGENT_TOKENS: {e}"))?;
            if !agent_tokens.is_empty() {
                tracing::info!(
                    count = agent_tokens.len(),
                    bindings = ?agent_tokens
                        .iter()
                        .map(|t| format!(
                            "{} (client={})",
                            t.from_agent,
                            t.client.as_deref().unwrap_or("-")
                        ))
                        .collect::<Vec<_>>(),
                    "agent tokens configured"
                );
            }

            // Scoped tokens are meaningless without a main token: dev mode
            // (no main token) classifies every caller as Full and returns
            // before the machine/agent scans, so configured scopes would
            // silently vanish and identity enforcement would fail open. Only
            // reachable via --dev-no-auth (a missing token otherwise aborts
            // above); refuse the contradictory config rather than pretend to
            // enforce.
            if main_token.is_none() && !(machine_tokens.is_empty() && agent_tokens.is_empty()) {
                anyhow::bail!(
                    "OPS_BRAIN_MACHINE_TOKENS / OPS_BRAIN_AGENT_TOKENS are set but no \
                     OPS_BRAIN_AUTH_TOKEN is configured — scoped tokens require a main token to \
                     enforce against. --dev-no-auth disables all scoping; drop the scoped tokens \
                     or set a real main token."
                );
            }

            let auth_state = auth::AuthState {
                main_token,
                machine_tokens: Arc::new(machine_tokens),
                agent_tokens: Arc::new(agent_tokens),
            };

            let api_routes = axum::Router::new()
                .route("/briefing", axum::routing::post(api::generate_briefing))
                .route("/handoff", axum::routing::post(api::create_handoff))
                .route("/pending", axum::routing::get(api::list_pending))
                .with_state(api_state);

            // Outer .layer wraps everything below — auth runs BEFORE rmcp's
            // host check inside /mcp. Don't reorder: unauthenticated callers
            // shouldn't be able to enumerate which Host values are accepted.
            let app = axum::Router::new()
                .route("/health", axum::routing::get(|| async { "OK" }))
                .nest("/api", api_routes)
                .nest_service("/mcp", mcp_service)
                .layer(axum::middleware::from_fn_with_state(
                    auth_state,
                    auth::bearer_auth,
                ));

            let listener = tokio::net::TcpListener::bind(&config.listen).await?;
            tracing::info!("Listening on http://{}", config.listen);
            axum::serve(listener, app).await?;
        }
        _ => {
            anyhow::bail!(
                "Unknown transport: {}. Use 'stdio' or 'http'.",
                config.transport
            );
        }
    }

    Ok(())
}
