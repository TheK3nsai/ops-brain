use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(name = "ops-brain", about = "Operational intelligence MCP server")]
pub struct Config {
    /// Database connection URL
    #[arg(long, env = "DATABASE_URL")]
    pub database_url: String,

    /// Transport mode: stdio or http
    #[arg(long, env = "OPS_BRAIN_TRANSPORT", default_value = "stdio")]
    pub transport: String,

    /// HTTP listen address (for http transport)
    #[arg(long, env = "OPS_BRAIN_LISTEN", default_value = "0.0.0.0:3000")]
    pub listen: String,

    /// Auth token (for http transport)
    #[arg(long, env = "OPS_BRAIN_AUTH_TOKEN")]
    pub auth_token: Option<String>,

    /// Run database migrations on startup
    #[arg(long, env = "OPS_BRAIN_MIGRATE", default_value = "true")]
    pub migrate: bool,
}
