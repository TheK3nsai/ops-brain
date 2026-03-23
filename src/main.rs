mod auth;
mod config;
mod db;
mod metrics;
mod models;
mod repo;
mod tools;

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

    let server = OpsBrain::new(pool.clone(), kuma_config.clone());

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

            let kuma_config_http = kuma_config.clone();
            let mcp_service = StreamableHttpService::new(
                move || Ok(OpsBrain::new(pool.clone(), kuma_config_http.clone())),
                session_manager,
                Default::default(),
            );

            let app = axum::Router::new()
                .route("/health", axum::routing::get(|| async { "OK" }))
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
