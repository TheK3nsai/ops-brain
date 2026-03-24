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
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("ops_brain=info")),
        )
        .init();

    let config = Config::parse();

    tracing::info!("Connecting to database...");
    let pool = db::create_pool(&config.database_url).await?;

    if config.migrate {
        tracing::info!("Running migrations...");
        db::run_migrations(&pool).await?;
    }

    let kuma_config = config.uptime_kuma_url.as_ref().map(|url| {
        tracing::info!("Uptime Kuma metrics configured: {}", url);
        metrics::UptimeKumaConfig {
            base_url: url.clone(),
            username: config.uptime_kuma_username.clone(),
            password: config.uptime_kuma_password.clone(),
        }
    });

    let zammad_config = match (&config.zammad_url, &config.zammad_api_token) {
        (Some(url), Some(token)) => {
            tracing::info!("Zammad integration configured: {}", url);
            Some(zammad::ZammadConfig {
                base_url: url.clone(),
                api_token: token.clone(),
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
        if let Some(ref kuma) = kuma_config {
            tracing::info!(
                interval = config.watchdog_interval_secs,
                "Starting proactive monitoring watchdog"
            );
            tokio::spawn(watchdog::run(
                pool.clone(),
                kuma.clone(),
                embedding_client.clone(),
                config.watchdog_interval_secs,
            ));
        } else {
            tracing::warn!(
                "Watchdog enabled but UPTIME_KUMA_URL not set — watchdog will not start"
            );
        }
    }

    let server = OpsBrain::new(pool.clone(), kuma_config.clone(), embedding_client.clone(), zammad_config.clone());

    match config.transport.as_str() {
        "stdio" => {
            tracing::info!("Starting ops-brain MCP server (stdio transport)");
            let transport = rmcp::transport::io::stdio();
            let service = server.serve(transport).await?;
            service.waiting().await?;
        }
        "http" => {
            use std::sync::Arc;
            use rmcp::transport::streamable_http_server::{
                session::local::LocalSessionManager,
                tower::StreamableHttpService,
            };

            let session_manager = Arc::new(LocalSessionManager::default());

            let api_state = Arc::new(api::ApiState {
                pool: pool.clone(),
                kuma_config: kuma_config.clone(),
                zammad_config: zammad_config.clone(),
            });

            let kuma_config_http = kuma_config.clone();
            let embedding_client_http = embedding_client.clone();
            let zammad_config_http = zammad_config.clone();
            let mcp_service = StreamableHttpService::new(
                move || Ok(OpsBrain::new(pool.clone(), kuma_config_http.clone(), embedding_client_http.clone(), zammad_config_http.clone())),
                session_manager,
                Default::default(),
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
