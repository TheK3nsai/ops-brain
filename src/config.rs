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

    /// Uptime Kuma base URL for metrics scraping (e.g. http://uptime-kuma:3001 or https://uptime.kensai.cloud)
    #[arg(long, env = "UPTIME_KUMA_URL")]
    pub uptime_kuma_url: Option<String>,

    /// Optional basic auth username for Uptime Kuma /metrics endpoint
    #[arg(long, env = "UPTIME_KUMA_USERNAME")]
    pub uptime_kuma_username: Option<String>,

    /// Optional basic auth password for Uptime Kuma /metrics endpoint
    #[arg(long, env = "UPTIME_KUMA_PASSWORD")]
    pub uptime_kuma_password: Option<String>,

    /// Zammad API base URL (e.g. http://zammad-railsserver:3000 or https://tickets.kensai.cloud)
    #[arg(long, env = "ZAMMAD_URL")]
    pub zammad_url: Option<String>,

    /// Zammad API token for authentication
    #[arg(long, env = "ZAMMAD_API_TOKEN")]
    pub zammad_api_token: Option<String>,

    /// Embedding API base URL (OpenAI-compatible). Default: local ollama.
    #[arg(
        long,
        env = "OPS_BRAIN_EMBEDDING_URL",
        default_value = "http://localhost:11434/v1/embeddings"
    )]
    pub embedding_url: String,

    /// Embedding model name (default: nomic-embed-text for ollama)
    #[arg(
        long,
        env = "OPS_BRAIN_EMBEDDING_MODEL",
        default_value = "nomic-embed-text"
    )]
    pub embedding_model: String,

    /// API key for embedding service (optional — not needed for local ollama)
    #[arg(long, env = "OPS_BRAIN_EMBEDDING_API_KEY")]
    pub embedding_api_key: Option<String>,

    /// Enable embedding generation (default: true if embedding URL is reachable)
    #[arg(long, env = "OPS_BRAIN_EMBEDDINGS_ENABLED")]
    pub embeddings_enabled: Option<bool>,

    /// Enable proactive monitoring watchdog (polls Uptime Kuma, auto-creates incidents)
    #[arg(long, env = "OPS_BRAIN_WATCHDOG_ENABLED", default_value = "false")]
    pub watchdog_enabled: bool,

    /// Watchdog polling interval in seconds (default: 60)
    #[arg(long, env = "OPS_BRAIN_WATCHDOG_INTERVAL", default_value = "60")]
    pub watchdog_interval_secs: u64,
}
