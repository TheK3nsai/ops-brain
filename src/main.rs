use clap::Parser;
use ops_brain::{api, auth, config::Config, db, oauth, reader, tools::OpsBrain};
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

    let instructions = ops_brain::tools::resolve_instructions(config.instructions_extra.as_deref())
        .map_err(anyhow::Error::msg)?;
    if instructions != ops_brain::tools::INSTRUCTIONS {
        tracing::info!(
            bytes = instructions.len(),
            "MCP instructions include a deployment addendum"
        );
    }

    match config.transport.as_str() {
        "stdio" => {
            tracing::info!("Starting ops-brain MCP server (stdio transport)");
            if config
                .reader_listen
                .as_deref()
                .is_some_and(|l| !l.trim().is_empty())
            {
                tracing::warn!(
                    "OPS_BRAIN_READER_LISTEN is ignored on the stdio transport; \
                     the bus reader runs with the http server"
                );
            }
            let server = OpsBrain::with_instructions(pool.clone(), instructions);
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
            // MCP clients don't auto-reinitialize on the resulting 404. Bumping
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
            // open dev server. Any non-blank token must meet MIN_TOKEN_LEN.
            let main_token: Option<String> =
                auth::validate_main_token(config.auth_token.as_deref())
                    .map_err(anyhow::Error::msg)?;
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
            // Per-agent tokens: identity-bound credentials for interactive MCP.
            // Cross-checked against the main + machine secrets so a shared
            // secret can't blur the credential classes. Parse failures abort
            // startup — a dropped agent token would read as "identity
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
                        .map(auth::AgentToken::binding_summary)
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

            // The bus reader is a separate listener with its own credential,
            // which must not repeat any bearer the server already accepts.
            let other_secrets: Vec<&str> = main_token
                .as_deref()
                .into_iter()
                .chain(machine_tokens.iter().map(|t| t.token.as_str()))
                .chain(agent_tokens.iter().map(|t| t.token.as_str()))
                .collect();
            let reader_config = reader::configure(
                config.reader_listen.as_deref(),
                config.reader_token.as_deref(),
                config.reader_operator.as_deref(),
                config.reader_utc_offset.as_deref(),
                &other_secrets,
            )
            .map_err(anyhow::Error::msg)?;
            let reader_pool = pool.clone();

            let auth_state = auth::AuthState {
                main_token,
                machine_tokens: Arc::new(machine_tokens),
                agent_tokens: Arc::new(agent_tokens),
            };

            let oauth = oauth::configured_state(
                config.oauth_config.as_deref(),
                config.oauth_resource_url.as_deref(),
                auth_state.main_token.is_some(),
            )
            .map_err(anyhow::Error::msg)?;

            // The curl quick start is "ready" as soon as the main bearer
            // exists. That bearer can file as any agent. Say so until the
            // operator configures identity-bound agent credentials.
            if auth_state.main_token.is_some()
                && auth_state.agent_tokens.is_empty()
                && oauth.is_none()
            {
                tracing::warn!(
                    "No per-agent tokens configured. The main bearer can file as any agent. \
                     Configure OPS_BRAIN_AGENT_TOKENS or OAuth before connecting agents, \
                     and keep the main bearer on the server."
                );
            }

            let session_instructions = instructions;
            // Who can read the bus, from the operator's static bindings: agent
            // tokens, OAuth bindings, and the queues read-scoped machine tokens poll.
            let known_agents: Vec<String> = auth_state
                .agent_tokens
                .iter()
                .map(|t| t.from_agent.clone())
                .chain(
                    auth_state
                        .machine_tokens
                        .iter()
                        .filter(|t| t.has_scope("read"))
                        .flat_map(|t| t.agents.iter().cloned()),
                )
                .chain(oauth.iter().flat_map(|o| o.bound_agents()))
                .collect();
            let mcp_cancel = http_config.cancellation_token.clone();
            let mcp_service = StreamableHttpService::new(
                move || {
                    Ok(
                        OpsBrain::with_instructions(pool.clone(), session_instructions.clone())
                            .with_known_agents(known_agents.clone()),
                    )
                },
                session_manager,
                http_config,
            );

            let api_routes = axum::Router::new()
                .route("/briefing", axum::routing::post(api::generate_briefing))
                .route("/handoff", axum::routing::post(api::create_handoff))
                .route("/pending", axum::routing::get(api::list_pending))
                .with_state(api_state.clone());

            // Outer .layer wraps everything below — auth runs BEFORE rmcp's
            // host check inside /mcp. Don't reorder: unauthenticated callers
            // shouldn't be able to enumerate which Host values are accepted.
            let app = axum::Router::new()
                .route("/health", axum::routing::get(|| async { "OK" }))
                .route("/ready", axum::routing::get(api::ready))
                .with_state(api_state)
                .nest("/api", api_routes)
                .nest_service("/mcp", mcp_service)
                .layer(axum::middleware::from_fn_with_state(
                    oauth::OAuthAuthState {
                        auth: auth_state,
                        oauth,
                    },
                    oauth::oauth_bearer_auth,
                ));

            let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
            let signal_tx = shutdown_tx.clone();
            let signal_cancel = mcp_cancel.clone();
            tokio::spawn(async move {
                shutdown_signal().await;
                // Cancel active MCP SSE streams so axum's graceful shutdown
                // drains promptly instead of waiting for idle clients until SIGKILL.
                signal_cancel.cancel();
                let _ = signal_tx.send(true);
            });

            let listener = tokio::net::TcpListener::bind(&config.listen).await?;
            tracing::info!("Listening on http://{}", config.listen);
            match reader_config {
                None => {
                    axum::serve(listener, app)
                        .with_graceful_shutdown(wait_for_shutdown(shutdown_rx))
                        .await?;
                }
                Some(reader_config) => {
                    // Bind before serving anything: a reader that was asked
                    // for and cannot start is a startup failure, not a warning.
                    let reader_listener = tokio::net::TcpListener::bind(&reader_config.listen)
                        .await
                        .map_err(|e| {
                            anyhow::anyhow!("OPS_BRAIN_READER_LISTEN={}: {e}", reader_config.listen)
                        })?;
                    tracing::info!(
                        listen = %reader_config.listen,
                        operator = %reader_config.operator,
                        token = reader_config.token.is_some(),
                        "Bus reader listening (read-only)"
                    );
                    if reader_config.token.is_none() {
                        tracing::warn!(
                            "Bus reader has no token: any local user or process that can \
                             reach its loopback port can read every handoff."
                        );
                    }
                    let reader_app = reader::router(reader_pool.clone(), reader_config);
                    // Either listener ending triggers shutdown of the other so
                    // a supervisor restarts both rather than leaving half a server up.
                    let main_tx = shutdown_tx.clone();
                    let main_cancel = mcp_cancel.clone();
                    let main_rx = shutdown_rx.clone();
                    let main_server = async move {
                        let res = axum::serve(listener, app)
                            .with_graceful_shutdown(wait_for_shutdown(main_rx))
                            .await;
                        main_cancel.cancel();
                        let _ = main_tx.send(true);
                        res
                    };
                    let reader_tx = shutdown_tx;
                    let reader_cancel = mcp_cancel;
                    let reader_rx = shutdown_rx;
                    let reader_server = async move {
                        let res = axum::serve(reader_listener, reader_app)
                            .with_graceful_shutdown(wait_for_shutdown(reader_rx))
                            .await;
                        reader_cancel.cancel();
                        let _ = reader_tx.send(true);
                        res
                    };
                    let (main_res, reader_res) = tokio::join!(main_server, reader_server);
                    main_res?;
                    reader_res?;
                }
            }
            reader_pool.close().await;
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

async fn wait_for_shutdown(mut rx: tokio::sync::watch::Receiver<bool>) {
    let _ = rx.wait_for(|v| *v).await;
}

/// Listen for SIGTERM (Docker / systemd stop) and SIGINT (Ctrl-C). Without an
/// explicit handler, Linux PID 1 inside a container ignores SIGTERM and hangs
/// until Docker sends SIGKILL.
async fn shutdown_signal() {
    let ctrl_c = async {
        if let Err(e) = tokio::signal::ctrl_c().await {
            tracing::error!(error = %e, "failed to listen for SIGINT");
            std::future::pending::<()>().await;
        }
    };

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::error!(error = %e, "failed to install SIGTERM handler");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("received SIGINT, shutting down"),
        _ = terminate => tracing::info!("received SIGTERM, shutting down"),
    }
}
