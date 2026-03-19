mod auth;
mod config;
mod db;
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

    let server = OpsBrain::new(pool);

    match config.transport.as_str() {
        "stdio" => {
            tracing::info!("Starting ops-brain MCP server (stdio transport)");
            let transport = rmcp::transport::io::stdio();
            let service = server.serve(transport).await?;
            service.waiting().await?;
        }
        _ => {
            anyhow::bail!(
                "Unknown transport: {}. Use 'stdio' (Phase 1) or 'http' (Phase 2)",
                config.transport
            );
        }
    }

    Ok(())
}
