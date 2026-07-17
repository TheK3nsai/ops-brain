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

    /// Machine tokens for the REST ingestion path (http transport only).
    /// JSON array of scoped token bindings, e.g.:
    /// `[{"token":"...","from_agent":"Example-Host1","client":"example",
    ///    "agents":["CC-Example"],"scopes":["create","read"]}]`
    /// Each token is limited to `POST /api/handoff` ("create" scope) and/or
    /// `GET /api/pending` ("read" scope) for the listed agents — never /mcp.
    #[arg(long, env = "OPS_BRAIN_MACHINE_TOKENS")]
    pub machine_tokens: Option<String>,

    /// Comma-separated allowed Host header values for HTTP transport (DNS-rebind mitigation
    /// added in rmcp 1.4). Defaults to loopback only — public deployments behind a reverse
    /// proxy must list their public hostname (e.g. `ops.example.com,ops.example.com:443`).
    #[arg(long, env = "OPS_BRAIN_ALLOWED_HOSTS")]
    pub allowed_hosts: Option<String>,

    /// Run database migrations on startup
    #[arg(long, env = "OPS_BRAIN_MIGRATE", default_value = "true")]
    pub migrate: bool,

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
}
