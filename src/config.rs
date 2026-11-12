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
}

#[cfg(test)]
mod tests {
    use super::*;

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
