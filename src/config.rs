use clap::{Parser, Subcommand};

#[derive(Subcommand, Debug, Clone)]
pub enum Command {
    /// Generate missing knowledge/handoff embeddings, then exit.
    BackfillEmbeddings {
        /// Specific table: knowledge or handoffs. Omit for both.
        #[arg(long)]
        table: Option<String>,
        /// Maximum records processed per table in this run.
        #[arg(long, default_value_t = 10, value_parser = clap::value_parser!(i64).range(1..=100))]
        batch_size: i64,
    },
}

#[derive(Parser, Debug, Clone)]
#[command(name = "ops-brain", about = "Operational intelligence MCP server")]
pub struct Config {
    #[command(subcommand)]
    pub command: Option<Command>,

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

    /// Explicitly serve HTTP without authentication. Without this flag, a
    /// missing or blank OPS_BRAIN_AUTH_TOKEN aborts startup on the http
    /// transport — an empty env var must never silently mean "open server".
    #[arg(long, env = "OPS_BRAIN_DEV_NO_AUTH", default_value_t = false)]
    pub dev_no_auth: bool,

    /// Machine tokens for the REST ingestion path (http transport only).
    /// JSON array of scoped token bindings, e.g.:
    /// `[{"token":"...","from_agent":"Example-Host1","client":"example",
    ///    "agents":["Claude-Example"],"scopes":["create","read"]}]`
    /// Each token is limited to `POST /api/handoff` ("create" scope) and/or
    /// `GET /api/pending` ("read" scope) for the listed agents — never /mcp.
    #[arg(long, env = "OPS_BRAIN_MACHINE_TOKENS")]
    pub machine_tokens: Option<String>,

    /// Per-agent tokens for interactive MCP sessions (http only).
    /// JSON array of identity-bound token bindings, e.g.:
    /// `[{"token":"...","from_agent":"Claude-Laptop","client":"acme"}]`
    /// Each token reaches `/mcp` only (never REST)
    /// with its `from_agent` bound server-side: MCP write tools reject a
    /// mismatching claimed identity. The main bearer stays unbound as operator
    /// break-glass. Secrets must be distinct from the main and machine tokens.
    #[arg(long, env = "OPS_BRAIN_AGENT_TOKENS")]
    pub agent_tokens: Option<String>,

    /// Optional external OAuth resource-server config (JSON); see docs/oauth.md.
    #[arg(long, env = "OPS_BRAIN_OAUTH_CONFIG")]
    pub oauth_config: Option<String>,

    /// Canonical public HTTPS MCP URL used for OAuth resource discovery.
    #[arg(long, env = "OPS_BRAIN_OAUTH_RESOURCE_URL")]
    pub oauth_resource_url: Option<String>,

    /// One deployment-local line appended to the built-in MCP instructions.
    /// Use it for a rule that is true of this operator's team and false of
    /// every other deployment. Blank omits it. See `tools::resolve_instructions`.
    #[arg(long, env = "OPS_BRAIN_INSTRUCTIONS_EXTRA")]
    pub instructions_extra: Option<String>,

    /// Comma-separated allowed Host header values for HTTP transport (DNS-rebind mitigation
    /// added in rmcp 1.4). Defaults to loopback only — public deployments behind a reverse
    /// proxy must list their public hostname (e.g. `ops.example.com,ops.example.com:443`).
    #[arg(long, env = "OPS_BRAIN_ALLOWED_HOSTS")]
    pub allowed_hosts: Option<String>,

    /// Run database migrations on startup
    #[arg(long, env = "OPS_BRAIN_MIGRATE", default_value = "true")]
    pub migrate: bool,

    /// Embedding API URL (OpenAI-compatible, e.g. local ollama at
    /// `http://localhost:11434/v1/embeddings`). Embeddings run only when this
    /// is set; blank counts as unset (Docker's MCP catalog passes empty values).
    #[arg(long, env = "OPS_BRAIN_EMBEDDING_URL")]
    pub embedding_url: Option<String>,

    /// Embedding model name (default: nomic-embed-text for ollama)
    /// Blank falls back to the default.
    #[arg(
        long,
        env = "OPS_BRAIN_EMBEDDING_MODEL",
        default_value = DEFAULT_EMBEDDING_MODEL,
        value_parser = parse_model
    )]
    pub embedding_model: String,

    /// API key for embedding service (optional — not needed for local ollama)
    #[arg(long, env = "OPS_BRAIN_EMBEDDING_API_KEY")]
    pub embedding_api_key: Option<String>,

    /// `false` turns embeddings off even when a URL is set. Blank means unset.
    #[arg(
        long,
        env = "OPS_BRAIN_EMBEDDINGS_ENABLED",
        default_value_t = true,
        action = clap::ArgAction::Set,
        value_parser = parse_enabled
    )]
    pub embeddings_enabled: bool,
}

impl Config {
    /// The embedding endpoint, or `None` when embeddings are off: no
    /// (or a blank) URL, or `OPS_BRAIN_EMBEDDINGS_ENABLED=false`.
    pub fn embedding_endpoint(&self) -> Option<&str> {
        let url = self.embedding_url.as_deref().map(str::trim);
        url.filter(|u| !u.is_empty() && self.embeddings_enabled)
    }
}

const DEFAULT_EMBEDDING_MODEL: &str = "nomic-embed-text";

fn parse_model(value: &str) -> Result<String, String> {
    let value = value.trim();
    Ok(if value.is_empty() {
        DEFAULT_EMBEDDING_MODEL
    } else {
        value
    }
    .to_string())
}

/// Blank is unset, which leaves the decision to the URL.
fn parse_enabled(value: &str) -> Result<bool, String> {
    match value.trim() {
        "" | "true" => Ok(true),
        "false" => Ok(false),
        other => Err(format!("expected true or false, got {other:?}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Every case passes both settings so CI's OPS_BRAIN_EMBEDDINGS_ENABLED
    // (or a developer's OPS_BRAIN_EMBEDDING_URL) cannot leak in.
    fn endpoint(url: &str, enabled: &str) -> Option<String> {
        let args = [
            "ops-brain",
            "--database-url",
            "postgres://test",
            "--embedding-url",
            url,
            "--embeddings-enabled",
            enabled,
        ];
        let config = Config::try_parse_from(args).unwrap();
        config.embedding_endpoint().map(str::to_string)
    }

    const URL: &str = "http://ollama:11434/v1/embeddings";

    #[test]
    fn blank_url_means_off() {
        assert_eq!(endpoint("", ""), None);
        assert_eq!(endpoint("  ", ""), None);
        assert_eq!(endpoint("", "true"), None);
    }

    #[test]
    fn url_means_on_unless_disabled() {
        assert_eq!(endpoint(URL, "").as_deref(), Some(URL));
        assert_eq!(endpoint(URL, "true").as_deref(), Some(URL));
        assert_eq!(endpoint(URL, "false"), None);
    }

    #[test]
    fn blank_model_falls_back_to_default() {
        let model = |m| {
            let args = ["ops-brain", "--database-url", "x", "--embedding-model", m];
            Config::try_parse_from(args).unwrap().embedding_model
        };
        assert_eq!(model(""), DEFAULT_EMBEDDING_MODEL);
        assert_eq!(model("  "), DEFAULT_EMBEDDING_MODEL);
        assert_eq!(model("mxbai-embed-large"), "mxbai-embed-large");
    }

    #[test]
    fn enabled_flag_rejects_junk() {
        assert!(parse_enabled("ture").is_err());
    }

    // The prod compose has no `env_file:`, so a var missing from its
    // `environment:` list never reaches the container, whatever `.env` says.
    #[test]
    fn prod_compose_passes_every_env_var() {
        use clap::CommandFactory;
        const NEVER_IN_PROD: &str = "OPS_BRAIN_DEV_NO_AUTH";
        let compose = include_str!("../docker-compose.prod.yml");
        let listed = |var: &str| {
            let entry = format!("- {var}=");
            compose.lines().any(|l| l.trim_start().starts_with(&entry))
        };
        let cmd = Config::command();
        let missing: Vec<String> = std::iter::once(&cmd)
            .chain(cmd.get_subcommands())
            .flat_map(|c| c.get_arguments())
            .filter_map(|arg| arg.get_env())
            .map(|env| env.to_string_lossy().into_owned())
            .filter(|var| var != NEVER_IN_PROD && !listed(var))
            .collect();
        assert!(
            missing.is_empty(),
            "add `- VAR=${{VAR:-}}` to docker-compose.prod.yml for: {missing:?}"
        );
        assert!(
            !compose.contains(NEVER_IN_PROD),
            "{NEVER_IN_PROD} must never appear in docker-compose.prod.yml, comments included"
        );
    }
}
