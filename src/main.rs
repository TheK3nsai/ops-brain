#![allow(dead_code)] // Repo/model functions used by integration tests via lib.rs

mod api;
mod auth;
mod config;
mod db;
mod embeddings;
mod metrics;
mod models;
mod repo;
mod tools;
mod validation;
mod watchdog;
mod zammad;

use clap::Parser;
use config::Config;
use rmcp::service::ServiceExt;
use tools::OpsBrain;
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

    let kuma_configs: Vec<metrics::UptimeKumaConfig> =
        if let Some(ref instances_json) = config.uptime_kuma_instances {
            // Multi-instance mode: parse JSON array
            #[derive(serde::Deserialize)]
            struct KumaInstance {
                name: String,
                url: String,
                username: Option<String>,
                password: Option<String>,
            }
            match serde_json::from_str::<Vec<KumaInstance>>(instances_json) {
                Ok(instances) => {
                    tracing::info!(
                        "Uptime Kuma: {} instance(s) configured via UPTIME_KUMA_INSTANCES",
                        instances.len()
                    );
                    instances
                        .into_iter()
                        .map(|i| {
                            tracing::info!("  - {} → {}", i.name, i.url);
                            metrics::UptimeKumaConfig {
                                name: i.name,
                                base_url: i.url,
                                username: i.username,
                                password: i.password,
                            }
                        })
                        .collect()
                }
                Err(e) => {
                    tracing::error!("Failed to parse UPTIME_KUMA_INSTANCES: {e}");
                    vec![]
                }
            }
        } else if let Some(ref url) = config.uptime_kuma_url {
            // Single-instance mode (backward compat)
            tracing::info!("Uptime Kuma metrics configured: {}", url);
            vec![metrics::UptimeKumaConfig {
                name: "default".to_string(),
                base_url: url.clone(),
                username: config.uptime_kuma_username.clone(),
                password: config.uptime_kuma_password.clone(),
            }]
        } else {
            vec![]
        };

    let zammad_config = match (&config.zammad_url, &config.zammad_api_token) {
        (Some(url), Some(token)) => {
            tracing::info!("Zammad integration configured: {}", url);
            Some(zammad::ZammadConfig {
                base_url: url.clone(),
                api_token: token.clone(),
                default_owner_id: config.zammad_default_owner_id,
            })
        }
        _ => {
            tracing::info!("Zammad not configured (set ZAMMAD_URL and ZAMMAD_API_TOKEN)");
            None
        }
    };

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

    // Spawn watchdog background task if enabled and Uptime Kuma is configured
    if config.watchdog_enabled {
        if !kuma_configs.is_empty() {
            let watchdog_config = watchdog::WatchdogConfig {
                interval_secs: config.watchdog_interval_secs,
                confirm_polls: config.watchdog_confirm_polls,
                cooldown_secs: config.watchdog_cooldown_secs,
                flap_threshold: config.watchdog_flap_threshold,
            };
            tracing::info!(
                interval = watchdog_config.interval_secs,
                confirm_polls = watchdog_config.confirm_polls,
                cooldown_secs = watchdog_config.cooldown_secs,
                instances = kuma_configs.len(),
                "Starting proactive monitoring watchdog"
            );
            tokio::spawn(watchdog::run(
                pool.clone(),
                kuma_configs.clone(),
                embedding_client.clone(),
                watchdog_config,
            ));
        } else {
            tracing::warn!(
                "Watchdog enabled but no Uptime Kuma instances configured — watchdog will not start"
            );
        }
    }

    let server = OpsBrain::new(
        pool.clone(),
        kuma_configs.clone(),
        embedding_client.clone(),
        zammad_config.clone(),
    );

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

            let session_manager = Arc::new(LocalSessionManager::default());

            let api_state = Arc::new(api::ApiState {
                pool: pool.clone(),
                kuma_configs: kuma_configs.clone(),
                zammad_config: zammad_config.clone(),
            });

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

            let kuma_configs_http = kuma_configs.clone();
            let embedding_client_http = embedding_client.clone();
            let zammad_config_http = zammad_config.clone();
            let mcp_service = StreamableHttpService::new(
                move || {
                    Ok(OpsBrain::new(
                        pool.clone(),
                        kuma_configs_http.clone(),
                        embedding_client_http.clone(),
                        zammad_config_http.clone(),
                    ))
                },
                session_manager,
                http_config,
            );

            let api_routes = axum::Router::new()
                .route("/briefing", axum::routing::post(api::generate_briefing))
                .with_state(api_state);

            let app = axum::Router::new()
                .route("/health", axum::routing::get(|| async { "OK" }))
                .nest("/api", api_routes)
                .nest_service("/mcp", mcp_service)
                .layer(axum::middleware::from_fn_with_state(
                    config.auth_token.clone(),
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
