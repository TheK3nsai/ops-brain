use axum::{extract::State, http::StatusCode, middleware::Next, response::Response};
use std::sync::Arc;

/// Constant-time token comparison to prevent timing attacks.
///
/// An empty `expected` never matches: a blank configured secret must not
/// turn `Authorization: Bearer ` (empty credential) into a valid login.
pub fn validate_token(token: &str, expected: &str) -> bool {
    if expected.is_empty() || token.len() != expected.len() {
        return false;
    }
    token
        .as_bytes()
        .iter()
        .zip(expected.as_bytes().iter())
        .fold(0u8, |acc, (a, b)| acc | (a ^ b))
        == 0
}

/// A scoped credential for non-interactive callers (monitors, cron sweeps,
/// wake shims). Machine tokens can reach exactly two endpoints — the REST
/// ingestion path and the pending poll — never `/mcp` or the rest of `/api`.
#[derive(Clone, serde::Deserialize)]
pub struct MachineToken {
    /// The bearer secret. Distinct from the main auth token by construction
    /// (validated at parse time).
    pub token: String,
    /// The `from_agent` stamped on every handoff this token files. Callers
    /// cannot override it — the binding IS the identity.
    pub from_agent: String,
    /// Informational client scope, recorded in logs for audit. Routing
    /// enforcement is `agents` (handoffs carry no client column).
    #[serde(default)]
    pub client: Option<String>,
    /// Agents this token may file handoffs to (`POST /api/handoff` to_agent)
    /// and poll pending queues for (`GET /api/pending` agent). Matched
    /// case-insensitively. Must be non-empty — there is no wildcard.
    #[serde(default)]
    pub agents: Vec<String>,
    /// Granted scopes: "create" (POST /api/handoff), "read" (GET /api/pending).
    #[serde(default)]
    pub scopes: Vec<String>,
}

/// Redacts the bearer secret. These structs land in request extensions (and,
/// for agent tokens, in rmcp's tool-call `Parts`), so a stray `?token`/`?ext`
/// debug log must never spill the credential.
impl std::fmt::Debug for MachineToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MachineToken")
            .field("token", &"<redacted>")
            .field("from_agent", &self.from_agent)
            .field("client", &self.client)
            .field("agents", &self.agents)
            .field("scopes", &self.scopes)
            .finish()
    }
}

impl MachineToken {
    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }

    /// Case-insensitive membership check against the token's agent allowlist.
    pub fn allows_agent(&self, agent: &str) -> bool {
        self.agents.iter().any(|a| a.eq_ignore_ascii_case(agent))
    }
}

pub const MACHINE_SCOPES: &[&str] = &["create", "read"];

/// Minimum machine-token length. These are minted secrets, not passwords —
/// anything short enough to brute-force is a config error.
const MIN_TOKEN_LEN: usize = 32;

/// Parse and validate the `OPS_BRAIN_MACHINE_TOKENS` JSON config.
///
/// Fails fast (startup abort) on any invalid entry: a half-valid token list
/// silently dropping entries would read as "monitor wired up" while its
/// filings 401.
pub fn parse_machine_tokens(
    raw: Option<&str>,
    main_token: Option<&str>,
) -> Result<Vec<MachineToken>, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };

    let tokens: Vec<MachineToken> = serde_json::from_str(raw)
        .map_err(|e| format!("OPS_BRAIN_MACHINE_TOKENS is not a valid JSON array: {e}"))?;

    for (i, t) in tokens.iter().enumerate() {
        if t.token.len() < MIN_TOKEN_LEN {
            return Err(format!(
                "machine token [{i}] is too short ({} chars, min {MIN_TOKEN_LEN})",
                t.token.len()
            ));
        }
        if let Some(main) = main_token {
            if t.token == main {
                return Err(format!(
                    "machine token [{i}] equals OPS_BRAIN_AUTH_TOKEN — machine tokens must be \
                     distinct secrets with a smaller blast radius"
                ));
            }
        }
        crate::validation::validate_agent_name(&t.from_agent)
            .map_err(|e| format!("machine token [{i}] from_agent: {e}"))?;
        if t.agents.is_empty() {
            return Err(format!(
                "machine token [{i}] ('{}') has an empty agents allowlist — list the agents it \
                 may file to / poll for explicitly; there is no wildcard",
                t.from_agent
            ));
        }
        for a in &t.agents {
            crate::validation::validate_agent_name(a)
                .map_err(|e| format!("machine token [{i}] agents: {e}"))?;
        }
        if t.scopes.is_empty() {
            return Err(format!(
                "machine token [{i}] ('{}') has no scopes — grant \"create\" and/or \"read\"",
                t.from_agent
            ));
        }
        for s in &t.scopes {
            if !MACHINE_SCOPES.contains(&s.as_str()) {
                return Err(format!(
                    "machine token [{i}] has unknown scope '{s}' (valid: {})",
                    MACHINE_SCOPES.join(", ")
                ));
            }
        }
    }

    // Duplicate secrets would make the first-match lookup ambiguous.
    for i in 0..tokens.len() {
        for j in (i + 1)..tokens.len() {
            if tokens[i].token == tokens[j].token {
                return Err(format!(
                    "machine tokens [{i}] and [{j}] share the same secret — mint one per machine"
                ));
            }
        }
    }

    Ok(tokens)
}

/// A per-agent credential for interactive MCP sessions. Where a machine token
/// is REST-only and never reaches `/mcp`, an agent token reaches the full MCP
/// tool surface — but its `from_agent` identity is bound server-side. Write
/// tools (`create_handoff`, `add_knowledge`) reject a mismatching claimed
/// identity, so the bus gains a sender guarantee the single shared main bearer
/// never had: a slug cannot appear on the bus without a token the operator
/// minted for it, and one exposure rotates one host, not the fleet.
#[derive(Clone, serde::Deserialize)]
pub struct AgentToken {
    /// The bearer secret. Distinct from the main token and every machine token
    /// by construction (validated at parse time).
    pub token: String,
    /// The agent identity this token is locked to. Every MCP write filed on
    /// this token must claim this slug (case-insensitive); reads that query a
    /// different slug are served but warn-logged as an anomaly. Normalized
    /// (trimmed) at parse time so it matches the trimmed claimed identity.
    pub from_agent: String,
    /// Informational client scope, recorded in logs for audit. Not yet
    /// enforced on MCP calls (tools carry an explicit `client_slug` param);
    /// reserved so a future per-client MCP binding needs no config change.
    #[serde(default)]
    pub client: Option<String>,
}

/// Redacts the bearer secret — this struct flows into rmcp's tool-call `Parts`.
impl std::fmt::Debug for AgentToken {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AgentToken")
            .field("token", &"<redacted>")
            .field("from_agent", &self.from_agent)
            .field("client", &self.client)
            .finish()
    }
}

/// Parse and validate the `OPS_BRAIN_AGENT_TOKENS` JSON config.
///
/// Fails fast (startup abort) on any invalid entry — a silently dropped agent
/// token would read as "identity enforced" while that agent still files
/// unbound. Every secret is cross-checked against the main token and all
/// machine tokens: a secret shared across classes would make the bearer_auth
/// first-match scan ambiguous and blur the blast-radius boundary the classes
/// exist to draw.
pub fn parse_agent_tokens(
    raw: Option<&str>,
    main_token: Option<&str>,
    machine_tokens: &[MachineToken],
) -> Result<Vec<AgentToken>, String> {
    let Some(raw) = raw.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(Vec::new());
    };

    let mut tokens: Vec<AgentToken> = serde_json::from_str(raw)
        .map_err(|e| format!("OPS_BRAIN_AGENT_TOKENS is not a valid JSON array: {e}"))?;

    for (i, t) in tokens.iter_mut().enumerate() {
        if t.token.len() < MIN_TOKEN_LEN {
            return Err(format!(
                "agent token [{i}] is too short ({} chars, min {MIN_TOKEN_LEN})",
                t.token.len()
            ));
        }
        if let Some(main) = main_token {
            if t.token == main {
                return Err(format!(
                    "agent token [{i}] equals OPS_BRAIN_AUTH_TOKEN — tokens must be distinct \
                     secrets so one exposure rotates one host, not the fleet"
                ));
            }
        }
        for (j, m) in machine_tokens.iter().enumerate() {
            if t.token == m.token {
                return Err(format!(
                    "agent token [{i}] shares a secret with machine token [{j}] ('{}') — each \
                     credential class must be a distinct secret",
                    m.from_agent
                ));
            }
        }
        // Store the normalized (trimmed) slug: identity enforcement compares it
        // against the trimmed claimed value with no further trimming, so a
        // padded config value would otherwise lock the agent out of filing as
        // itself.
        t.from_agent = crate::validation::validate_agent_name(&t.from_agent)
            .map_err(|e| format!("agent token [{i}] from_agent: {e}"))?
            .to_string();
    }

    // Duplicate secrets would make the first-match lookup ambiguous.
    for i in 0..tokens.len() {
        for j in (i + 1)..tokens.len() {
            if tokens[i].token == tokens[j].token {
                return Err(format!(
                    "agent tokens [{i}] and [{j}] share the same secret — mint one per agent"
                ));
            }
        }
    }

    Ok(tokens)
}

/// Enforce a write tool's claimed identity against the caller's server-bound
/// agent, if any.
///
/// - `bound = None` (main bearer, or stdio/dev where no HTTP auth ran) → always
///   `Ok`; unbound callers keep the pre-tokens behavior. On the public HTTP
///   listener every authenticated request carries a class, so `None` here only
///   arises in trusted-local contexts.
/// - `bound = Some(id)` and the claim matches (case-insensitive) → `Ok`.
/// - `bound = Some(id)` and the claim differs → rejected: a per-agent token may
///   only write as the slug it was minted for.
pub fn check_bound_identity(bound: Option<&str>, claimed: &str) -> Result<(), String> {
    match bound {
        None => Ok(()),
        Some(id) if id.eq_ignore_ascii_case(claimed) => Ok(()),
        Some(id) => Err(format!(
            "identity '{claimed}' does not match your token-bound agent '{id}'. A per-agent \
             token may only write as itself — set the field to '{id}' (or omit it)."
        )),
    }
}

/// Warn-log when a bound caller queries a read tool for a *different* agent's
/// data. Reads are never blocked — cross-agent reads are legitimate during
/// triage — but "token bound to X is reading Y's queue" is an anomaly worth
/// surfacing in logs. No-op for unbound callers and for self-queries.
pub fn warn_identity_mismatch(bound: Option<&str>, queried: &str, tool: &str) {
    if let Some(id) = bound {
        if !id.eq_ignore_ascii_case(queried) {
            tracing::warn!(
                bound = %id,
                queried = %queried,
                tool = %tool,
                "agent token queried another agent's data"
            );
        }
    }
}

/// Who is calling, resolved by the auth middleware and stored in request
/// extensions for handlers that differentiate (the machine endpoints and the
/// identity-bound MCP write tools).
#[derive(Debug, Clone)]
pub enum CallerClass {
    /// Main bearer (or auth disabled in dev) — full surface, unbound identity.
    Full,
    /// A machine token — scoped to the machine endpoints it was granted.
    Machine(Arc<MachineToken>),
    /// A per-agent token — reaches `/mcp` only, with a server-bound identity.
    Agent(Arc<AgentToken>),
}

impl CallerClass {
    /// The server-bound agent identity this caller is locked to, if any. Only
    /// a per-agent token binds an MCP identity; `Full` (main bearer / dev) and
    /// `Machine` (REST-only, never reaches `/mcp`) return `None`, meaning "no
    /// MCP identity enforcement".
    pub fn bound_agent(&self) -> Option<&str> {
        match self {
            CallerClass::Agent(t) => Some(&t.from_agent),
            CallerClass::Full | CallerClass::Machine(_) => None,
        }
    }
}

#[derive(Clone)]
pub struct AuthState {
    pub main_token: Option<String>,
    pub machine_tokens: Arc<Vec<MachineToken>>,
    pub agent_tokens: Arc<Vec<AgentToken>>,
}

/// The (method, path) pairs a machine token may reach, and the scope each
/// requires. Everything else is 403 for machine callers.
fn required_machine_scope(method: &axum::http::Method, path: &str) -> Option<&'static str> {
    match (method.as_str(), path) {
        ("POST", "/api/handoff") => Some("create"),
        ("GET", "/api/pending") => Some("read"),
        _ => None,
    }
}

/// Axum middleware: validates Bearer token on all non-health requests.
///
/// Main bearer → full access (CallerClass::Full). Machine token → only its
/// granted machine endpoints (CallerClass::Machine). No token configured at
/// all → dev mode, allow as Full.
pub async fn bearer_auth(
    State(state): State<AuthState>,
    mut request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Health endpoint is always public
    if request.uri().path() == "/health" {
        return Ok(next.run(request).await);
    }

    // If no main token configured, allow all requests (dev mode). Machine
    // tokens without a main token would be scoped credentials on an open
    // server — pointless, so dev mode wins.
    let Some(ref expected) = state.main_token else {
        request.extensions_mut().insert(CallerClass::Full);
        return Ok(next.run(request).await);
    };

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    let Some(presented) = auth_header.and_then(|h| h.strip_prefix("Bearer ")) else {
        return Err(StatusCode::UNAUTHORIZED);
    };

    if validate_token(presented, expected) {
        request.extensions_mut().insert(CallerClass::Full);
        return Ok(next.run(request).await);
    }

    // Scan machine tokens. Constant-time per comparison; the scan itself
    // leaks only the (public) count of configured tokens. Secrets are unique
    // across classes (enforced at parse time), so match order is immaterial.
    if let Some(token) = state
        .machine_tokens
        .iter()
        .find(|t| validate_token(presented, &t.token))
    {
        let method = request.method().clone();
        let path = request.uri().path().to_string();
        let Some(needed) = required_machine_scope(&method, &path) else {
            tracing::warn!(
                from_agent = %token.from_agent,
                %path,
                "machine token attempted a non-machine endpoint"
            );
            return Err(StatusCode::FORBIDDEN);
        };
        if !token.has_scope(needed) {
            tracing::warn!(
                from_agent = %token.from_agent,
                %path,
                scope = needed,
                "machine token lacks required scope"
            );
            return Err(StatusCode::FORBIDDEN);
        }

        request
            .extensions_mut()
            .insert(CallerClass::Machine(Arc::new(token.clone())));
        return Ok(next.run(request).await);
    }

    // Scan per-agent tokens. These reach the MCP surface only — the identity
    // binding is enforced inside the tool handlers, not at this layer, so the
    // gate here is purely "is this an MCP request". Everything else is 403,
    // symmetric to the machine tokens' REST-only restriction.
    if let Some(token) = state
        .agent_tokens
        .iter()
        .find(|t| validate_token(presented, &t.token))
    {
        let path = request.uri().path().to_string();
        if path == "/mcp" || path.starts_with("/mcp/") {
            request
                .extensions_mut()
                .insert(CallerClass::Agent(Arc::new(token.clone())));
            return Ok(next.run(request).await);
        }
        tracing::warn!(
            from_agent = %token.from_agent,
            %path,
            "agent token attempted a non-MCP endpoint"
        );
        return Err(StatusCode::FORBIDDEN);
    }

    Err(StatusCode::UNAUTHORIZED)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_token_correct() {
        assert!(validate_token("my-secret-token", "my-secret-token"));
    }

    #[test]
    fn validate_token_wrong() {
        assert!(!validate_token("wrong-token", "my-secret-token"));
    }

    #[test]
    fn validate_token_different_lengths() {
        assert!(!validate_token("short", "my-secret-token"));
        assert!(!validate_token("my-secret-token-longer", "my-secret-token"));
    }

    #[test]
    fn validate_token_empty() {
        assert!(!validate_token("", "my-secret-token"));
        assert!(!validate_token("something", ""));
        // A blank configured secret must never validate anything — not even
        // an equally blank presented credential.
        assert!(!validate_token("", ""));
    }

    #[test]
    fn validate_token_single_char_diff() {
        assert!(!validate_token("aaaa", "aaab"));
        assert!(!validate_token("aaab", "aaaa"));
    }

    // parse_machine_tokens

    const T: &str = "0123456789abcdef0123456789abcdef"; // 32 chars

    fn entry(token: &str) -> String {
        format!(
            r#"{{"token":"{token}","from_agent":"Example-Host1","client":"example",
                "agents":["CC-Example"],"scopes":["create","read"]}}"#
        )
    }

    #[test]
    fn parse_none_and_empty_are_ok() {
        assert!(parse_machine_tokens(None, None).unwrap().is_empty());
        assert!(parse_machine_tokens(Some(""), None).unwrap().is_empty());
        assert!(parse_machine_tokens(Some("  "), None).unwrap().is_empty());
    }

    #[test]
    fn parse_valid_entry() {
        let raw = format!("[{}]", entry(T));
        let tokens = parse_machine_tokens(Some(&raw), Some("main-token")).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].from_agent, "Example-Host1");
        assert!(tokens[0].has_scope("create"));
        assert!(tokens[0].has_scope("read"));
        assert!(tokens[0].allows_agent("cc-example")); // case-insensitive
        assert!(!tokens[0].allows_agent("CC-Other"));
    }

    #[test]
    fn parse_rejects_invalid_json() {
        assert!(parse_machine_tokens(Some("not json"), None).is_err());
        assert!(parse_machine_tokens(Some(r#"{"token":"x"}"#), None).is_err()); // not an array
    }

    #[test]
    fn parse_rejects_short_token() {
        let raw = format!("[{}]", entry("short"));
        let err = parse_machine_tokens(Some(&raw), None).unwrap_err();
        assert!(err.contains("too short"));
    }

    #[test]
    fn parse_rejects_token_equal_to_main() {
        let raw = format!("[{}]", entry(T));
        let err = parse_machine_tokens(Some(&raw), Some(T)).unwrap_err();
        assert!(err.contains("OPS_BRAIN_AUTH_TOKEN"));
    }

    #[test]
    fn parse_rejects_empty_agents() {
        let raw = format!(
            r#"[{{"token":"{T}","from_agent":"Example-Host1","agents":[],"scopes":["create"]}}]"#
        );
        let err = parse_machine_tokens(Some(&raw), None).unwrap_err();
        assert!(err.contains("agents allowlist"));
    }

    #[test]
    fn parse_rejects_empty_and_unknown_scopes() {
        let raw = format!(
            r#"[{{"token":"{T}","from_agent":"Example-Host1","agents":["CC-Example"],"scopes":[]}}]"#
        );
        assert!(parse_machine_tokens(Some(&raw), None)
            .unwrap_err()
            .contains("no scopes"));

        let raw = format!(
            r#"[{{"token":"{T}","from_agent":"Example-Host1","agents":["CC-Example"],"scopes":["admin"]}}]"#
        );
        assert!(parse_machine_tokens(Some(&raw), None)
            .unwrap_err()
            .contains("unknown scope"));
    }

    #[test]
    fn parse_rejects_duplicate_secrets() {
        let raw = format!("[{},{}]", entry(T), entry(T));
        let err = parse_machine_tokens(Some(&raw), None).unwrap_err();
        assert!(err.contains("same secret"));
    }

    #[test]
    fn parse_rejects_bad_agent_names() {
        let raw = format!(
            r#"[{{"token":"{T}","from_agent":"bad agent","agents":["CC-Example"],"scopes":["create"]}}]"#
        );
        assert!(parse_machine_tokens(Some(&raw), None).is_err());
    }

    // scope routing table

    #[test]
    fn machine_scope_table() {
        use axum::http::Method;
        assert_eq!(
            required_machine_scope(&Method::POST, "/api/handoff"),
            Some("create")
        );
        assert_eq!(
            required_machine_scope(&Method::GET, "/api/pending"),
            Some("read")
        );
        assert_eq!(required_machine_scope(&Method::GET, "/api/handoff"), None);
        assert_eq!(required_machine_scope(&Method::POST, "/api/pending"), None);
        assert_eq!(required_machine_scope(&Method::POST, "/api/briefing"), None);
        assert_eq!(required_machine_scope(&Method::POST, "/mcp"), None);
    }

    // parse_agent_tokens

    const A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"; // 32 chars, distinct from T

    fn agent_entry(token: &str) -> String {
        format!(r#"{{"token":"{token}","from_agent":"CC-Example","client":"example"}}"#)
    }

    fn machine_list() -> Vec<MachineToken> {
        parse_machine_tokens(Some(&format!("[{}]", entry(T))), None).unwrap()
    }

    #[test]
    fn parse_agent_none_and_empty_are_ok() {
        assert!(parse_agent_tokens(None, None, &[]).unwrap().is_empty());
        assert!(parse_agent_tokens(Some("  "), None, &[])
            .unwrap()
            .is_empty());
    }

    #[test]
    fn parse_agent_valid_entry() {
        let raw = format!("[{}]", agent_entry(A));
        let tokens = parse_agent_tokens(Some(&raw), Some("main-token"), &[]).unwrap();
        assert_eq!(tokens.len(), 1);
        assert_eq!(tokens[0].from_agent, "CC-Example");
        assert_eq!(tokens[0].client.as_deref(), Some("example"));
    }

    #[test]
    fn parse_agent_rejects_short_token() {
        let raw = format!("[{}]", agent_entry("short"));
        assert!(parse_agent_tokens(Some(&raw), None, &[])
            .unwrap_err()
            .contains("too short"));
    }

    #[test]
    fn parse_agent_rejects_token_equal_to_main() {
        let raw = format!("[{}]", agent_entry(A));
        assert!(parse_agent_tokens(Some(&raw), Some(A), &[])
            .unwrap_err()
            .contains("OPS_BRAIN_AUTH_TOKEN"));
    }

    #[test]
    fn parse_agent_rejects_secret_shared_with_machine_token() {
        // Agent token reusing a machine token's secret must fail — distinct
        // classes must be distinct secrets.
        let raw = format!("[{}]", agent_entry(T));
        let err = parse_agent_tokens(Some(&raw), None, &machine_list()).unwrap_err();
        assert!(err.contains("machine token"), "got: {err}");
    }

    #[test]
    fn parse_agent_rejects_duplicate_secrets() {
        let raw = format!("[{},{}]", agent_entry(A), agent_entry(A));
        assert!(parse_agent_tokens(Some(&raw), None, &[])
            .unwrap_err()
            .contains("same secret"));
    }

    #[test]
    fn parse_agent_rejects_bad_agent_name() {
        let raw = format!("[{}]", agent_entry_named(A, "bad agent"));
        assert!(parse_agent_tokens(Some(&raw), None, &[]).is_err());
    }

    #[test]
    fn parse_agent_normalizes_padded_from_agent() {
        // A padded config slug must be stored trimmed, or check_bound_identity
        // (which compares against the trimmed claim with no further trim) would
        // lock the agent out of filing as itself.
        let raw = format!("[{}]", agent_entry_named(A, "  CC-Stealth  "));
        let tokens = parse_agent_tokens(Some(&raw), None, &[]).unwrap();
        assert_eq!(tokens[0].from_agent, "CC-Stealth");
        assert!(check_bound_identity(Some(&tokens[0].from_agent), "CC-Stealth").is_ok());
    }

    fn agent_entry_named(token: &str, from_agent: &str) -> String {
        format!(r#"{{"token":"{token}","from_agent":"{from_agent}"}}"#)
    }

    // check_bound_identity

    #[test]
    fn bound_identity_unbound_always_ok() {
        assert!(check_bound_identity(None, "anyone").is_ok());
    }

    #[test]
    fn bound_identity_matches_case_insensitively() {
        assert!(check_bound_identity(Some("CC-Stealth"), "cc-stealth").is_ok());
        assert!(check_bound_identity(Some("CC-Stealth"), "CC-Stealth").is_ok());
    }

    #[test]
    fn bound_identity_rejects_mismatch() {
        let err = check_bound_identity(Some("CC-Stealth"), "CC-Cloud").unwrap_err();
        assert!(err.contains("CC-Stealth"));
        assert!(err.contains("CC-Cloud"));
    }

    // CallerClass::bound_agent

    #[test]
    fn caller_class_bound_agent() {
        let agent = AgentToken {
            token: A.to_string(),
            from_agent: "CC-Example".to_string(),
            client: None,
        };
        assert_eq!(
            CallerClass::Agent(Arc::new(agent)).bound_agent(),
            Some("CC-Example")
        );
        assert_eq!(CallerClass::Full.bound_agent(), None);
        let machine = parse_machine_tokens(Some(&format!("[{}]", entry(T))), None).unwrap();
        assert_eq!(
            CallerClass::Machine(Arc::new(machine.into_iter().next().unwrap())).bound_agent(),
            None
        );
    }
}
