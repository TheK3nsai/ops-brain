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

    /// Comma-separated allowed Host header values for HTTP transport (DNS-rebind mitigation
    /// added in rmcp 1.4). Defaults to loopback only — public deployments behind a reverse
    /// proxy must list their public hostname (e.g. `ops.example.com,ops.example.com:443`).
    #[arg(long, env = "OPS_BRAIN_ALLOWED_HOSTS")]
    pub allowed_hosts: Option<String>,

    /// Run database migrations on startup
    #[arg(long, env = "OPS_BRAIN_MIGRATE", default_value = "true")]
    pub migrate: bool,

    /// Uptime Kuma base URL for metrics scraping (e.g. http://uptime-kuma:3001 or https://uptime.example.com).
    /// For a single instance. Use UPTIME_KUMA_INSTANCES for multiple instances.
    #[arg(long, env = "UPTIME_KUMA_URL")]
    pub uptime_kuma_url: Option<String>,

    /// Optional basic auth username for Uptime Kuma /metrics endpoint (single-instance mode)
    #[arg(long, env = "UPTIME_KUMA_USERNAME")]
    pub uptime_kuma_username: Option<String>,

    /// Optional basic auth password for Uptime Kuma /metrics endpoint (single-instance mode)
    #[arg(long, env = "UPTIME_KUMA_PASSWORD")]
    pub uptime_kuma_password: Option<String>,

    /// Multiple Uptime Kuma instances as JSON array. Takes precedence over UPTIME_KUMA_URL.
    /// Format: [{"name":"cloud","url":"http://kuma:3001"},{"name":"lab","url":"http://10.0.0.1:3001","username":"user","password":"pass"}]
    #[arg(long, env = "UPTIME_KUMA_INSTANCES")]
    pub uptime_kuma_instances: Option<String>,

    /// Zammad API base URL (e.g. http://zammad-railsserver:3000 or https://tickets.example.com)
    #[arg(long, env = "ZAMMAD_URL")]
    pub zammad_url: Option<String>,

    /// Zammad API token for authentication
    #[arg(long, env = "ZAMMAD_API_TOKEN")]
    pub zammad_api_token: Option<String>,

    /// Zammad default ticket owner user ID (optional — omit to leave unassigned)
    #[arg(long, env = "ZAMMAD_DEFAULT_OWNER_ID")]
    pub zammad_default_owner_id: Option<i64>,

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

    /// Number of consecutive DOWN polls before creating an incident (flap suppression).
    /// With default interval of 60s, a value of 3 means a monitor must be down for ~3 minutes.
    #[arg(long, env = "OPS_BRAIN_WATCHDOG_CONFIRM_POLLS", default_value = "3")]
    pub watchdog_confirm_polls: u32,

    /// Cooldown in seconds after resolving an incident before creating a new one for the same
    /// monitor (flap suppression). Default: 1800 (30 minutes).
    #[arg(long, env = "OPS_BRAIN_WATCHDOG_COOLDOWN_SECS", default_value = "1800")]
    pub watchdog_cooldown_secs: u64,

    /// Global chronic flapper threshold. When a reopened incident's recurrence_count reaches
    /// this value, severity auto-downgrades to "low". At 2x threshold, the incident is
    /// auto-resolved immediately. Per-monitor flap_threshold (via link_monitor) overrides this.
    /// Default: 5.
    #[arg(long, env = "OPS_BRAIN_WATCHDOG_FLAP_THRESHOLD", default_value = "5")]
    pub watchdog_flap_threshold: u32,
}
