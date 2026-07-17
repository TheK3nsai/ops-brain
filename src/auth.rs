use axum::{extract::State, http::StatusCode, middleware::Next, response::Response};
use std::sync::Arc;

/// Constant-time token comparison to prevent timing attacks.
pub fn validate_token(token: &str, expected: &str) -> bool {
    if token.len() != expected.len() {
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
#[derive(Debug, Clone, serde::Deserialize)]
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

/// Who is calling, resolved by the auth middleware and stored in request
/// extensions for handlers that differentiate (the machine endpoints).
#[derive(Debug, Clone)]
pub enum CallerClass {
    /// Main bearer (or auth disabled in dev) — full surface.
    Full,
    /// A machine token — scoped to the machine endpoints it was granted.
    Machine(Arc<MachineToken>),
}

#[derive(Clone)]
pub struct AuthState {
    pub main_token: Option<String>,
    pub machine_tokens: Arc<Vec<MachineToken>>,
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
    // leaks only the (public) count of configured tokens.
    let matched = state
        .machine_tokens
        .iter()
        .find(|t| validate_token(presented, &t.token));

    let Some(token) = matched else {
        return Err(StatusCode::UNAUTHORIZED);
    };

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
    Ok(next.run(request).await)
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
        assert!(validate_token("", "")); // both empty = match
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
}
