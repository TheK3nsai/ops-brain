//! Optional OAuth resource server. The external IdP owns authorization-code,
//! PKCE, client registration and token issuance; ops-brain only verifies access
//! tokens and maps an explicitly allowed principal to an existing agent seat.
//!
//! Tokens must follow RFC 9068 (`typ: at+jwt`, `client_id`, resource audience).
//! ID tokens, opaque tokens and HMAC JWTs are deliberately unsupported.

use std::{
    collections::HashSet,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::State,
    http::{header, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use serde::Deserialize;
use tokio::sync::Mutex;

use crate::{
    api::ApiError,
    auth::{self, AgentToken, AuthState, CallerClass},
};

const MAX_TOKEN_BYTES: usize = 16 * 1024;
const MAX_JWKS_BYTES: usize = 256 * 1024;
const MAX_JWKS_KEYS: usize = 64;
const CACHE_TTL: Duration = Duration::from_secs(300);
const REFRESH_COOLDOWN: Duration = Duration::from_secs(30);

/// Enable OAuth only when both public settings and operator authentication
/// are present. Blank Compose interpolation has the same meaning as unset.
pub fn configured_state(
    raw: Option<&str>,
    resource: Option<&str>,
    authenticated: bool,
) -> Result<Option<Arc<OAuthState>>, String> {
    let present = |value: &str| !value.trim().is_empty();
    match (raw.filter(|v| present(v)), resource.filter(|v| present(v))) {
        (None, None) => Ok(None),
        (Some(raw), Some(resource)) if authenticated => {
            OAuthState::from_json(raw, resource).map(|state| Some(Arc::new(state)))
        }
        (Some(_), Some(_)) => {
            Err("OAuth requires authenticated HTTP mode; configure OPS_BRAIN_AUTH_TOKEN".into())
        }
        _ => Err("set both OPS_BRAIN_OAUTH_CONFIG and OPS_BRAIN_OAUTH_RESOURCE_URL".into()),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuthConfig {
    issuer: String,
    jwks_uri: String,
    /// Optional private CA certificate (PEM). Adds trust for the configured
    /// JWKS endpoint without disabling hostname or certificate validation.
    #[serde(default)]
    jwks_ca_pem: Option<String>,
    /// Defaults to the public resource URL. This must identify this resource,
    /// never an OAuth application's client ID or another API.
    audience: Option<String>,
    #[serde(default)]
    required_scopes: Vec<String>,
    bindings: Vec<OAuthBinding>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct OAuthBinding {
    subject: String,
    #[serde(default)]
    oauth_client_id: Option<String>,
    from_agent: String,
    #[serde(default)]
    client: Option<String>,
    #[serde(default)]
    exclude_client_data: bool,
}

struct VerificationKey {
    kid: String,
    algorithm: Algorithm,
    key: DecodingKey,
}

#[derive(Default)]
struct JwksCache {
    keys: Vec<VerificationKey>,
    fetched_at: Option<Instant>,
    last_attempt: Option<Instant>,
}

/// Share one instance across the HTTP listener so concurrent unknown-key
/// requests coalesce into a bounded refresh rather than a JWKS fetch storm.
pub struct OAuthState {
    config: OAuthConfig,
    resource_url: String,
    metadata_path: String,
    metadata_url: String,
    http: reqwest::Client,
    cache: Mutex<JwksCache>,
}

/// Existing static authentication state remains unchanged for current callers
/// and integrations. Configuring OAuth must never enable the open dev mode.
#[derive(Clone)]
pub struct OAuthAuthState {
    pub auth: AuthState,
    pub oauth: Option<Arc<OAuthState>>,
}

#[derive(Debug, PartialEq, Eq)]
enum OAuthError {
    InvalidToken,
    UnmappedPrincipal,
    InsufficientScope,
}

#[derive(Deserialize)]
struct Claims {
    #[serde(rename = "iss")]
    _issuer: String,
    sub: String,
    exp: u64,
    iat: u64,
    jti: String,
    #[serde(default, rename = "nbf")]
    _nbf: Option<u64>,
    client_id: String,
    #[serde(default)]
    scope: String,
}

fn https_url(value: &str, label: &str) -> Result<reqwest::Url, String> {
    let url =
        reqwest::Url::parse(value).map_err(|_| format!("{label} must be an absolute HTTPS URL"))?;
    if url.scheme() != "https"
        || url.host_str().is_none()
        || !url.username().is_empty()
        || url.password().is_some()
        || url.query().is_some()
        || url.fragment().is_some()
        || value
            .bytes()
            .any(|b| b <= 0x20 || b == b'"' || b == b'\\' || b == 0x7f)
    {
        return Err(format!(
            "{label} must be an HTTPS URL without credentials, query or fragment"
        ));
    }
    Ok(url)
}

fn valid_scope(scope: &str) -> bool {
    !scope.is_empty()
        && scope
            .bytes()
            .all(|b| b == 0x21 || (0x23..=0x5b).contains(&b) || (0x5d..=0x7e).contains(&b))
}

impl OAuthState {
    /// Parse public OAuth configuration; errors intentionally omit raw config
    /// and principal values. No network access or credentials are needed here.
    pub fn from_json(raw: &str, resource_url: &str) -> Result<Self, String> {
        let mut config: OAuthConfig = serde_json::from_str(raw).map_err(|e| {
            format!(
                "OAuth config is invalid JSON (line {}, column {})",
                e.line(),
                e.column()
            )
        })?;
        let resource = https_url(resource_url, "OAuth public resource")?;
        https_url(&config.issuer, "OAuth issuer")?;
        https_url(&config.jwks_uri, "OAuth JWKS URI")?;
        if config
            .audience
            .as_ref()
            .is_some_and(|a| a.trim().is_empty())
        {
            return Err("OAuth audience must not be blank".into());
        }
        if config.bindings.is_empty() {
            return Err("OAuth requires at least one explicit principal binding".into());
        }
        for binding in &mut config.bindings {
            if binding.subject.trim().is_empty()
                || binding
                    .oauth_client_id
                    .as_ref()
                    .is_some_and(|s| s.trim().is_empty())
            {
                return Err(
                    "OAuth binding subject and optional OAuth client ID must not be blank".into(),
                );
            }
            binding.from_agent = crate::validation::validate_agent_name(&binding.from_agent)
                .map_err(|_| "OAuth binding has an invalid from_agent".to_string())?
                .to_string();
        }
        for (i, left) in config.bindings.iter().enumerate() {
            for right in &config.bindings[i + 1..] {
                if left.subject == right.subject
                    && (left.oauth_client_id.is_none()
                        || right.oauth_client_id.is_none()
                        || left.oauth_client_id == right.oauth_client_id)
                {
                    return Err("OAuth principal bindings overlap; use distinct subjects or explicit OAuth client IDs".into());
                }
            }
        }
        if config.required_scopes.iter().any(|s| !valid_scope(s)) {
            return Err("OAuth required_scopes contains an invalid OAuth scope token".into());
        }
        let metadata_path = format!(
            "/.well-known/oauth-protected-resource{}",
            resource.path().trim_end_matches('/')
        );
        let mut metadata = resource;
        metadata.set_path(&metadata_path);
        let mut http = reqwest::Client::builder()
            .https_only(true)
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(3))
            .timeout(Duration::from_secs(5));
        if let Some(pem) = &config.jwks_ca_pem {
            let certificate = reqwest::Certificate::from_pem(pem.as_bytes())
                .map_err(|_| "OAuth JWKS CA must be a PEM certificate".to_string())?;
            http = http.add_root_certificate(certificate);
        }
        let http = http
            .build()
            .map_err(|_| "unable to build OAuth JWKS HTTP client".to_string())?;
        Ok(Self {
            config,
            resource_url: resource_url.to_string(),
            metadata_path,
            metadata_url: metadata.to_string(),
            http,
            cache: Mutex::new(JwksCache::default()),
        })
    }

    pub fn metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "resource": self.resource_url,
            "authorization_servers": [self.config.issuer],
            "scopes_supported": self.config.required_scopes,
            "bearer_methods_supported": ["header"]
        })
    }

    pub fn metadata_paths(&self) -> [&str; 2] {
        ["/.well-known/oauth-protected-resource", &self.metadata_path]
    }

    fn binding(&self, subject: &str, client_id: &str) -> Option<&OAuthBinding> {
        self.config.bindings.iter().find(|b| {
            b.subject == subject && b.oauth_client_id.as_deref().is_none_or(|c| c == client_id)
        })
    }

    fn challenge(&self, error: Option<&str>) -> HeaderValue {
        // All interpolated values are validated public config or constants.
        let mut challenge = format!("Bearer resource_metadata=\"{}\"", self.metadata_url);
        if !self.config.required_scopes.is_empty() {
            challenge.push_str(&format!(
                ", scope=\"{}\"",
                self.config.required_scopes.join(" ")
            ));
        }
        if let Some(error) = error {
            challenge.push_str(&format!(", error=\"{error}\""));
        }
        HeaderValue::from_str(&challenge).expect("OAuth challenge uses validated public values")
    }

    fn rejection(&self, error: OAuthError) -> Response {
        // Consent cannot create a server-side allowlist binding. Do not send
        // an insufficient_scope challenge that would start a login loop.
        if error == OAuthError::UnmappedPrincipal {
            return ApiError::forbidden("OAuth principal has no configured agent binding")
                .into_response();
        }
        let (status, code, message) = match error {
            OAuthError::InvalidToken => (
                StatusCode::UNAUTHORIZED,
                "invalid_token",
                "OAuth access token is invalid or expired",
            ),
            OAuthError::UnmappedPrincipal => unreachable!("handled above"),
            OAuthError::InsufficientScope => (
                StatusCode::FORBIDDEN,
                "insufficient_scope",
                "OAuth access token lacks required scopes",
            ),
        };
        let mut response = ApiError::new(status, message).into_response();
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, self.challenge(Some(code)));
        response
    }

    async fn validate(&self, token: &str) -> Result<AgentToken, OAuthError> {
        if token.len() > MAX_TOKEN_BYTES {
            return Err(OAuthError::InvalidToken);
        }
        let header = decode_header(token).map_err(|_| OAuthError::InvalidToken)?;
        // jsonwebtoken ignores unknown JOSE header fields. We implement no
        // critical extensions (including detached/unencoded payloads), so
        // their presence must fail rather than silently changing semantics.
        let raw_header = token.split('.').next().ok_or(OAuthError::InvalidToken)?;
        let raw_header = URL_SAFE_NO_PAD
            .decode(raw_header)
            .map_err(|_| OAuthError::InvalidToken)?;
        let raw_header: serde_json::Value =
            serde_json::from_slice(&raw_header).map_err(|_| OAuthError::InvalidToken)?;
        if raw_header.get("crit").is_some() || raw_header.get("b64").is_some() {
            return Err(OAuthError::InvalidToken);
        }
        if !matches!(header.alg, Algorithm::RS256 | Algorithm::ES256)
            || !header.typ.as_deref().is_some_and(|typ| {
                typ.eq_ignore_ascii_case("at+jwt") || typ.eq_ignore_ascii_case("application/at+jwt")
            })
        {
            return Err(OAuthError::InvalidToken);
        }
        // Ignore jku/x5u/jwk or any other header-provided key locations. Only
        // this operator-configured HTTPS JWKS URI may supply public keys.
        let kid = header
            .kid
            .as_deref()
            .filter(|k| !k.is_empty() && k.len() <= 256)
            .ok_or(OAuthError::InvalidToken)?;
        let key = self.key(kid, header.alg).await?;
        let mut validation = Validation::new(header.alg);
        validation.set_issuer(&[&self.config.issuer]);
        validation.set_audience(&[self
            .config
            .audience
            .as_deref()
            .unwrap_or(&self.resource_url)]);
        validation.set_required_spec_claims(&["iss", "sub", "aud", "exp"]);
        validation.leeway = 0;
        validation.validate_nbf = true;
        let claims = decode::<Claims>(token, &key, &validation)
            .map_err(|_| OAuthError::InvalidToken)?
            .claims;
        if claims.exp <= jsonwebtoken::get_current_timestamp()
            || claims.iat > jsonwebtoken::get_current_timestamp()
            || claims.exp <= claims.iat
            || claims.jti.trim().is_empty()
            || claims.sub.trim().is_empty()
            || claims.client_id.trim().is_empty()
        {
            return Err(OAuthError::InvalidToken);
        }
        let binding = self
            .binding(&claims.sub, &claims.client_id)
            .ok_or(OAuthError::UnmappedPrincipal)?;
        let scopes: HashSet<_> = claims.scope.split_ascii_whitespace().collect();
        if !self
            .config
            .required_scopes
            .iter()
            .all(|s| scopes.contains(s.as_str()))
        {
            return Err(OAuthError::InsufficientScope);
        }
        Ok(agent_from_binding(binding))
    }

    async fn key(&self, kid: &str, algorithm: Algorithm) -> Result<DecodingKey, OAuthError> {
        self.key_with_fetch(kid, algorithm, || self.fetch_keys())
            .await
    }

    async fn key_with_fetch<F, Fut>(
        &self,
        kid: &str,
        algorithm: Algorithm,
        fetch: F,
    ) -> Result<DecodingKey, OAuthError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<Vec<VerificationKey>, OAuthError>>,
    {
        let mut cache = self.cache.lock().await;
        let now = Instant::now();
        let fresh = cache
            .fetched_at
            .is_some_and(|at| now.duration_since(at) < CACHE_TTL);
        if fresh {
            if let Some(key) = cache
                .keys
                .iter()
                .find(|k| k.kid == kid && k.algorithm == algorithm)
            {
                return Ok(key.key.clone());
            }
        }
        // Bound negative lookups as well as failed fetches. The lock is held
        // over the request so concurrent rotations trigger one refresh.
        if cache
            .last_attempt
            .is_some_and(|at| now.duration_since(at) < REFRESH_COOLDOWN)
        {
            return Err(OAuthError::InvalidToken);
        }
        cache.last_attempt = Some(now);
        let keys = fetch().await?;
        cache.keys = keys;
        cache.fetched_at = Some(Instant::now());
        cache
            .keys
            .iter()
            .find(|k| k.kid == kid && k.algorithm == algorithm)
            .map(|k| k.key.clone())
            .ok_or(OAuthError::InvalidToken)
    }

    async fn fetch_keys(&self) -> Result<Vec<VerificationKey>, OAuthError> {
        let mut response = self
            .http
            .get(&self.config.jwks_uri)
            .send()
            .await
            .map_err(|_| OAuthError::InvalidToken)?
            .error_for_status()
            .map_err(|_| OAuthError::InvalidToken)?;
        if response
            .content_length()
            .is_some_and(|size| size > MAX_JWKS_BYTES as u64)
        {
            return Err(OAuthError::InvalidToken);
        }
        let mut body = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|_| OAuthError::InvalidToken)?
        {
            if body.len() + chunk.len() > MAX_JWKS_BYTES {
                return Err(OAuthError::InvalidToken);
            }
            body.extend_from_slice(&chunk);
        }
        parse_keys(&body)
    }
}

fn agent_from_binding(binding: &OAuthBinding) -> AgentToken {
    AgentToken {
        token: String::new(),
        from_agent: binding.from_agent.clone(),
        client: binding.client.clone(),
        exclude_client_data: binding.exclude_client_data,
    }
}

fn parse_keys(body: &[u8]) -> Result<Vec<VerificationKey>, OAuthError> {
    if body.len() > MAX_JWKS_BYTES {
        return Err(OAuthError::InvalidToken);
    }
    let jwks: serde_json::Value =
        serde_json::from_slice(body).map_err(|_| OAuthError::InvalidToken)?;
    let keys = jwks
        .get("keys")
        .and_then(|v| v.as_array())
        .ok_or(OAuthError::InvalidToken)?;
    if keys.is_empty() || keys.len() > MAX_JWKS_KEYS {
        return Err(OAuthError::InvalidToken);
    }
    let mut parsed = Vec::new();
    let mut ids = HashSet::new();
    for key in keys {
        // Encryption keys, HMAC keys and unsupported algorithms are never
        // candidates. An omitted JWK alg is safe because kty/curve bind it.
        if key.get("use").is_some_and(|v| v.as_str() != Some("sig")) {
            continue;
        }
        if key.get("key_ops").is_some_and(|v| {
            !v.as_array()
                .is_some_and(|ops| ops.iter().any(|op| op.as_str() == Some("verify")))
        }) {
            continue;
        }
        let algorithm = match (
            key.get("kty").and_then(|v| v.as_str()),
            key.get("crv").and_then(|v| v.as_str()),
        ) {
            (Some("RSA"), _) => Algorithm::RS256,
            (Some("EC"), Some("P-256")) => Algorithm::ES256,
            _ => continue,
        };
        let expected_alg = match algorithm {
            Algorithm::RS256 => "RS256",
            _ => "ES256",
        };
        if key
            .get("alg")
            .is_some_and(|v| v.as_str() != Some(expected_alg))
        {
            continue;
        }
        let kid = key
            .get("kid")
            .and_then(|v| v.as_str())
            .filter(|k| !k.is_empty() && k.len() <= 256)
            .ok_or(OAuthError::InvalidToken)?;
        if !ids.insert(kid.to_string()) {
            return Err(OAuthError::InvalidToken);
        }
        let jwk: jsonwebtoken::jwk::Jwk =
            serde_json::from_value(key.clone()).map_err(|_| OAuthError::InvalidToken)?;
        let decoding_key = DecodingKey::from_jwk(&jwk).map_err(|_| OAuthError::InvalidToken)?;
        parsed.push(VerificationKey {
            kid: kid.to_string(),
            algorithm,
            key: decoding_key,
        });
    }
    if parsed.is_empty() {
        return Err(OAuthError::InvalidToken);
    }
    Ok(parsed)
}

/// Wrap the existing auth middleware. Static tokens keep their exact classes
/// and endpoint restrictions; only unmatched MCP credentials enter OAuth.
/// Metadata is public at the RFC 9728 resource path and root fallback path.
pub async fn oauth_bearer_auth(
    State(state): State<OAuthAuthState>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    let Some(oauth) = &state.oauth else {
        return auth::bearer_auth(State(state.auth), request, next)
            .await
            .into_response();
    };
    let path = request.uri().path();
    if oauth.metadata_paths().contains(&path)
        && matches!(*request.method(), Method::GET | Method::HEAD)
    {
        let mut response = Json(oauth.metadata()).into_response();
        response.headers_mut().insert(
            header::CACHE_CONTROL,
            HeaderValue::from_static("public, max-age=300"),
        );
        if request.method() == Method::HEAD {
            *response.body_mut() = axum::body::Body::empty();
        }
        return response;
    }
    if matches!(path, "/health" | "/ready") {
        return next.run(request).await;
    }
    // Defense in depth: even an embedding application constructing this
    // state incorrectly must not let OAuth + dev-no-auth grant Full access.
    if state
        .auth
        .main_token
        .as_deref()
        .is_none_or(|t| t.is_empty())
    {
        return ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "OAuth requires static operator authentication to be configured",
        )
        .into_response();
    }
    let presented = request.headers().get_all(header::AUTHORIZATION);
    let mut headers = presented.iter();
    let token = headers
        .next()
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.strip_prefix("Bearer "));
    if headers.next().is_some() {
        return oauth.rejection(OAuthError::InvalidToken);
    }
    if let Some(token) = token {
        let is_static = state
            .auth
            .main_token
            .as_deref()
            .is_some_and(|t| auth::validate_token(token, t))
            || state
                .auth
                .agent_tokens
                .iter()
                .any(|t| auth::validate_token(token, &t.token))
            || state
                .auth
                .machine_tokens
                .iter()
                .any(|t| auth::validate_token(token, &t.token));
        if is_static {
            return auth::bearer_auth(State(state.auth), request, next)
                .await
                .into_response();
        }
        if path == "/mcp" || path.starts_with("/mcp/") {
            return match oauth.validate(token).await {
                Ok(agent) => {
                    request
                        .extensions_mut()
                        .insert(CallerClass::Agent(Arc::new(agent)));
                    next.run(request).await
                }
                Err(error) => oauth.rejection(error),
            };
        }
    }
    // OAuth tokens never enter REST authentication, even if otherwise valid.
    // Add discovery only on MCP responses, not the machine/operator surfaces.
    let is_mcp = path == "/mcp" || path.starts_with("/mcp/");
    let mut response = auth::bearer_auth(State(state.auth), request, next)
        .await
        .into_response();
    if is_mcp && response.status() == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, oauth.challenge(None));
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{to_bytes, Body},
        extract::Extension,
        http::Request,
        routing::post,
        Router,
    };
    use jsonwebtoken::{encode, EncodingKey, Header};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    struct FixtureKeys {
        encoding: EncodingKey,
        jwks: String,
    }

    // Fresh keys per test process: no private fixture material in the repo.
    static FIXTURE: std::sync::LazyLock<FixtureKeys> = std::sync::LazyLock::new(|| {
        use ring::{
            rand::SystemRandom,
            signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_FIXED_SIGNING},
        };
        let rng = SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng)
            .unwrap();
        let public = pair.public_key().as_ref();
        FixtureKeys {
            encoding: EncodingKey::from_ec_der(pkcs8.as_ref()),
            jwks: json!({"keys":[{"kty":"EC","crv":"P-256","kid":"test-key","use":"sig","alg":"ES256",
                "x":URL_SAFE_NO_PAD.encode(&public[1..33]),"y":URL_SAFE_NO_PAD.encode(&public[33..65])}]}).to_string(),
        }
    });
    const MAIN_TOKEN: &str = "operator-fixture-aaaaaaaaaaaaaaaaaaaaaaaa";
    const AGENT_TOKEN: &str = "agent-fixture-aaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const MACHINE_TOKEN: &str = "machine-fixture-aaaaaaaaaaaaaaaaaaaaaaaaa";
    const RESOURCE: &str = "https://bus.example.com/mcp";

    fn config() -> Value {
        json!({
            "issuer":"https://id.example.com/tenant",
            "jwks_uri":"https://id.example.com/tenant/keys",
            "required_scopes":["mcp"],
            "bindings":[{"subject":"opaque-user-id","oauth_client_id":"approved-client","from_agent":"Test-Bot","exclude_client_data":true}]
        })
    }

    fn state() -> OAuthState {
        let state = OAuthState::from_json(&config().to_string(), RESOURCE).unwrap();
        let now = Instant::now();
        *state.cache.try_lock().unwrap() = JwksCache {
            keys: parse_keys(FIXTURE.jwks.as_bytes()).unwrap(),
            fetched_at: Some(now),
            last_attempt: Some(now),
        };
        state
    }

    fn claims() -> Value {
        json!({"iss":"https://id.example.com/tenant","sub":"opaque-user-id","aud":RESOURCE,"exp":jsonwebtoken::get_current_timestamp()+3600,"iat":jsonwebtoken::get_current_timestamp(),"jti":"fixture-token-id","client_id":"approved-client","scope":"mcp other"})
    }

    fn signed_with_header(claims: &Value, header: Header) -> String {
        encode(&header, claims, &FIXTURE.encoding).unwrap()
    }

    fn access_header() -> Header {
        let mut header = Header::new(Algorithm::ES256);
        header.typ = Some("at+jwt".into());
        header.kid = Some("test-key".into());
        header
    }

    fn signed(claims: &Value) -> String {
        signed_with_header(claims, access_header())
    }

    async fn assert_invalid(state: &OAuthState, claims: &Value) {
        assert!(matches!(
            state.validate(&signed(claims)).await,
            Err(OAuthError::InvalidToken)
        ));
    }

    #[test]
    fn config_requires_explicit_unambiguous_bindings() {
        let mut value = config();
        value["bindings"] = json!([]);
        assert!(OAuthState::from_json(&value.to_string(), RESOURCE).is_err());
        value = config();
        let duplicate = value["bindings"][0].clone();
        value["bindings"].as_array_mut().unwrap().push(duplicate);
        assert!(OAuthState::from_json(&value.to_string(), RESOURCE).is_err());
        value["bindings"][1]["oauth_client_id"] = json!("different-client");
        assert!(OAuthState::from_json(&value.to_string(), RESOURCE).is_ok());
        value["bindings"][1]
            .as_object_mut()
            .unwrap()
            .remove("oauth_client_id");
        assert!(OAuthState::from_json(&value.to_string(), RESOURCE).is_err());
    }

    #[test]
    fn startup_requires_complete_authenticated_oauth_configuration() {
        let raw = config().to_string();
        assert!(configured_state(None, None, false).unwrap().is_none());
        assert!(configured_state(Some(" "), Some(""), true)
            .unwrap()
            .is_none());
        assert!(configured_state(Some(&raw), None, true).is_err());
        assert!(configured_state(None, Some(RESOURCE), true).is_err());
        assert!(configured_state(Some(&raw), Some(RESOURCE), false).is_err());
        assert!(configured_state(Some(&raw), Some(RESOURCE), true)
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn one_subject_can_hold_distinct_client_bound_seats() {
        let mut value = config();
        let mut second = value["bindings"][0].clone();
        second["oauth_client_id"] = json!("other-client");
        second["from_agent"] = json!("Other-Bot");
        second["exclude_client_data"] = json!(false);
        value["bindings"].as_array_mut().unwrap().push(second);
        let state = OAuthState::from_json(&value.to_string(), RESOURCE).unwrap();
        let now = Instant::now();
        *state.cache.lock().await = JwksCache {
            keys: parse_keys(FIXTURE.jwks.as_bytes()).unwrap(),
            fetched_at: Some(now),
            last_attempt: Some(now),
        };
        let first = state.validate(&signed(&claims())).await.unwrap();
        let mut other = claims();
        other["client_id"] = json!("other-client");
        let second = state.validate(&signed(&other)).await.unwrap();
        assert_eq!(first.from_agent, "Test-Bot");
        assert!(first.exclude_client_data);
        assert_eq!(second.from_agent, "Other-Bot");
        assert!(!second.exclude_client_data);
    }

    #[test]
    fn config_rejects_typos_and_unsafe_discovery_urls() {
        for field in ["issuer", "jwks_uri"] {
            for url in [
                "http://id.example.com/keys",
                "https://user:secret@id.example.com",
                "https://id.example.com?query=1",
                "https://id.example.com#fragment",
            ] {
                let mut value = config();
                value[field] = json!(url);
                assert!(OAuthState::from_json(&value.to_string(), RESOURCE).is_err());
            }
        }
        for resource in [
            "http://bus.example.com/mcp",
            "https://bus.example.com/mcp?query=1",
            "https://bus.example.com/mcp#frag",
            "https://bus.example.com/\"bad",
        ] {
            assert!(OAuthState::from_json(&config().to_string(), resource).is_err());
        }
        let mut value = config();
        value["bindings"][0]["excludeClientData"] = json!(true);
        assert!(OAuthState::from_json(&value.to_string(), RESOURCE).is_err());
        value = config();
        value["required_scopes"] = json!(["mcp\"\r\nunsafe"]);
        assert!(OAuthState::from_json(&value.to_string(), RESOURCE).is_err());
    }

    #[tokio::test]
    async fn verified_access_token_maps_to_existing_restricted_agent() {
        let state = state();
        let agent = state.validate(&signed(&claims())).await.unwrap();
        assert_eq!(agent.from_agent, "Test-Bot");
        assert!(agent.exclude_client_data);
        assert!(
            agent.token.is_empty(),
            "the bearer is never retained in extensions"
        );
        let caller = CallerClass::Agent(Arc::new(agent));
        assert_eq!(caller.bound_agent(), Some("Test-Bot"));
        assert_eq!(caller.restricted_agent(), Some("Test-Bot"));
    }

    #[tokio::test]
    async fn validates_signature_issuer_audience_expiry_and_not_before() {
        let state = state();
        for (field, value) in [
            ("iss", json!("https://other.example.com")),
            ("aud", json!("other-api")),
            ("exp", json!(jsonwebtoken::get_current_timestamp() - 1)),
            ("nbf", json!(jsonwebtoken::get_current_timestamp() + 3600)),
            ("nbf", json!("invalid-timestamp")),
            ("iat", json!(jsonwebtoken::get_current_timestamp() + 3600)),
            ("jti", json!("")),
            ("client_id", json!("")),
            ("sub", json!("")),
        ] {
            let mut value_claims = claims();
            value_claims[field] = value;
            assert_invalid(&state, &value_claims).await;
        }
        for field in ["iss", "aud", "exp", "sub", "client_id", "iat", "jti"] {
            let mut value = claims();
            value.as_object_mut().unwrap().remove(field);
            assert_invalid(&state, &value).await;
        }
        let token = signed(&claims());
        let (prefix, signature) = token.rsplit_once('.').unwrap();
        let changed = if signature.starts_with('a') { 'b' } else { 'a' };
        let tampered = format!("{prefix}.{changed}{}", &signature[1..]);
        assert!(matches!(
            state.validate(&tampered).await,
            Err(OAuthError::InvalidToken)
        ));
    }

    #[tokio::test]
    async fn rejects_id_tokens_hmac_missing_type_and_missing_or_unknown_keys() {
        let state = state();
        for typ in [None, Some("JWT".to_string()), Some("id+jwt".to_string())] {
            let mut header = access_header();
            header.typ = typ;
            assert!(matches!(
                state.validate(&signed_with_header(&claims(), header)).await,
                Err(OAuthError::InvalidToken)
            ));
        }
        for kid in [None, Some("unknown-key".to_string())] {
            let mut header = access_header();
            header.kid = kid;
            assert!(matches!(
                state.validate(&signed_with_header(&claims(), header)).await,
                Err(OAuthError::InvalidToken)
            ));
        }
        let mut header = Header::new(Algorithm::HS256);
        header.typ = Some("at+jwt".into());
        header.kid = Some("test-key".into());
        let token = encode(&header, &claims(), &EncodingKey::from_secret(b"wrong-key")).unwrap();
        assert!(matches!(
            state.validate(&token).await,
            Err(OAuthError::InvalidToken)
        ));
        assert!(matches!(
            state.validate(&"a".repeat(MAX_TOKEN_BYTES + 1)).await,
            Err(OAuthError::InvalidToken)
        ));
        // Extra header fields can be signed by the fixture but are not
        // interpreted by this resource server, so critical ones must fail.
        for extra in [json!({"crit":["unknown"]}), json!({"b64":false})] {
            let mut header = serde_json::to_value(access_header()).unwrap();
            header
                .as_object_mut()
                .unwrap()
                .extend(extra.as_object().unwrap().clone());
            let header = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&header).unwrap());
            let payload = URL_SAFE_NO_PAD.encode(serde_json::to_vec(&claims()).unwrap());
            let message = format!("{header}.{payload}");
            let signature =
                jsonwebtoken::crypto::sign(message.as_bytes(), &FIXTURE.encoding, Algorithm::ES256)
                    .unwrap();
            assert!(matches!(
                state.validate(&format!("{message}.{signature}")).await,
                Err(OAuthError::InvalidToken)
            ));
        }
    }

    #[tokio::test]
    async fn exact_subject_client_binding_and_scopes_are_required() {
        let state = state();
        for field in ["sub", "client_id"] {
            let mut value = claims();
            value[field] = json!("not-allowlisted");
            assert!(matches!(
                state.validate(&signed(&value)).await,
                Err(OAuthError::UnmappedPrincipal)
            ));
        }
        let mut value = claims();
        value["scope"] = json!("other");
        assert!(matches!(
            state.validate(&signed(&value)).await,
            Err(OAuthError::InsufficientScope)
        ));
        value["scope"] = json!("mcp-extra");
        assert!(matches!(
            state.validate(&signed(&value)).await,
            Err(OAuthError::InsufficientScope)
        ));
    }

    #[tokio::test]
    async fn stale_keys_fail_closed_and_unknown_kids_respect_global_refresh_cooldown() {
        let state = state();
        let last_attempt = state.cache.lock().await.last_attempt;
        for _ in 0..5 {
            assert!(state.key("missing", Algorithm::ES256).await.is_err());
        }
        assert_eq!(state.cache.lock().await.last_attempt, last_attempt);
        state.cache.lock().await.fetched_at =
            Some(Instant::now() - CACHE_TTL - Duration::from_secs(1));
        assert_invalid(&state, &claims()).await;
    }

    #[tokio::test]
    async fn rotated_keys_refresh_once_for_concurrent_requests_without_external_io() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let state = state();
        state.cache.lock().await.last_attempt = Some(Instant::now() - REFRESH_COOLDOWN);
        let calls = AtomicUsize::new(0);
        let refresh = || async {
            calls.fetch_add(1, Ordering::SeqCst);
            tokio::task::yield_now().await;
            parse_keys(FIXTURE.jwks.replace("test-key", "rotated-key").as_bytes())
        };
        let (left, right) = tokio::join!(
            state.key_with_fetch("rotated-key", Algorithm::ES256, refresh),
            state.key_with_fetch("rotated-key", Algorithm::ES256, refresh)
        );
        assert!(left.is_ok());
        assert!(right.is_ok());
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        assert!(state.key("test-key", Algorithm::ES256).await.is_err());
    }

    #[tokio::test]
    async fn failed_refresh_never_uses_expired_cache_or_retries_inside_cooldown() {
        let state = state();
        {
            let mut cache = state.cache.lock().await;
            cache.fetched_at = Some(Instant::now() - CACHE_TTL);
            cache.last_attempt = Some(Instant::now() - REFRESH_COOLDOWN);
        }
        let result = state
            .key_with_fetch("test-key", Algorithm::ES256, || async {
                Err(OAuthError::InvalidToken)
            })
            .await;
        assert!(result.is_err());
        assert_invalid(&state, &claims()).await;
        let result = state
            .key_with_fetch("test-key", Algorithm::ES256, || async {
                panic!("provider must not be retried inside the global cooldown")
            })
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn jwks_rejects_duplicate_or_inappropriate_keys_and_bounds_size() {
        let value: Value = serde_json::from_str(&FIXTURE.jwks).unwrap();
        let key = value["keys"][0].clone();
        for replacement in [
            json!({"keys":[key.clone(),key.clone()]}),
            json!({"keys":[]}),
            json!({"keys":vec![key.clone();MAX_JWKS_KEYS+1]}),
        ] {
            assert!(parse_keys(replacement.to_string().as_bytes()).is_err());
        }
        for (field, value) in [
            ("use", json!("enc")),
            ("key_ops", json!(["sign"])),
            ("alg", json!("HS256")),
            ("kid", Value::Null),
            ("crv", json!("P-384")),
        ] {
            let mut edited = key.clone();
            edited[field] = value;
            assert!(parse_keys(json!({"keys":[edited]}).to_string().as_bytes()).is_err());
        }
        assert!(parse_keys(&vec![b' '; MAX_JWKS_BYTES + 1]).is_err());
    }

    async fn echo(Extension(caller): Extension<CallerClass>) -> Json<Value> {
        Json(json!({"agent":caller.bound_agent(),"restricted":caller.restricted_agent()}))
    }

    fn app(oauth: Option<OAuthState>, main_token: Option<&str>) -> Router {
        let auth = AuthState {
            main_token: main_token.map(str::to_string),
            agent_tokens: Arc::new(vec![AgentToken {
                token: AGENT_TOKEN.into(),
                from_agent: "Static-Bot".into(),
                client: None,
                exclude_client_data: true,
            }]),
            machine_tokens: Arc::new(vec![auth::MachineToken {
                token: MACHINE_TOKEN.into(),
                from_agent: "Machine-Bot".into(),
                client: None,
                agents: vec!["Static-Bot".into()],
                scopes: vec!["read".into()],
            }]),
        };
        Router::new()
            .route("/mcp", post(echo))
            .route("/api/briefing", post(echo))
            .layer(axum::middleware::from_fn_with_state(
                OAuthAuthState {
                    auth,
                    oauth: oauth.map(Arc::new),
                },
                oauth_bearer_auth,
            ))
    }

    #[tokio::test]
    async fn middleware_metadata_and_discovery_are_public_only_when_enabled() {
        for path in [
            "/.well-known/oauth-protected-resource",
            "/.well-known/oauth-protected-resource/mcp",
        ] {
            let response = app(Some(state()), Some(MAIN_TOKEN))
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body: Value =
                serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                    .unwrap();
            assert_eq!(body["resource"], RESOURCE);
            assert_eq!(
                body["authorization_servers"][0],
                "https://id.example.com/tenant"
            );
        }
        let response = app(Some(state()), Some(MAIN_TOKEN))
            .oneshot(Request::post("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        let challenge = response.headers()[header::WWW_AUTHENTICATE]
            .to_str()
            .unwrap();
        assert!(challenge.contains("resource_metadata=\"https://bus.example.com/.well-known/oauth-protected-resource/mcp\""));
        assert!(challenge.contains("scope=\"mcp\""));
        let response = app(None, Some(MAIN_TOKEN))
            .oneshot(
                Request::get("/.well-known/oauth-protected-resource")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn middleware_preserves_static_classes_and_restricts_oauth_to_mcp() {
        for (token, path, status, expected_agent) in [
            (signed(&claims()), "/mcp", StatusCode::OK, Some("Test-Bot")),
            (
                AGENT_TOKEN.to_string(),
                "/mcp",
                StatusCode::OK,
                Some("Static-Bot"),
            ),
            (
                MAIN_TOKEN.to_string(),
                "/api/briefing",
                StatusCode::OK,
                None,
            ),
            (
                MACHINE_TOKEN.to_string(),
                "/mcp",
                StatusCode::FORBIDDEN,
                None,
            ),
            (
                AGENT_TOKEN.to_string(),
                "/api/briefing",
                StatusCode::FORBIDDEN,
                None,
            ),
            (
                signed(&claims()),
                "/api/briefing",
                StatusCode::UNAUTHORIZED,
                None,
            ),
        ] {
            let response = app(Some(state()), Some(MAIN_TOKEN))
                .oneshot(
                    Request::post(path)
                        .header(header::AUTHORIZATION, format!("Bearer {token}"))
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            assert_eq!(response.status(), status, "path {path}");
            if status == StatusCode::OK {
                let body: Value =
                    serde_json::from_slice(&to_bytes(response.into_body(), 4096).await.unwrap())
                        .unwrap();
                assert_eq!(body["agent"], json!(expected_agent));
            }
        }
    }

    #[tokio::test]
    async fn middleware_fails_closed_on_dev_configuration_and_duplicate_authorization() {
        let response = app(Some(state()), None)
            .oneshot(Request::post("/mcp").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
        let request = Request::post("/mcp")
            .header(header::AUTHORIZATION, format!("Bearer {MAIN_TOKEN}"))
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", signed(&claims())),
            )
            .body(Body::empty())
            .unwrap();
        let response = app(Some(state()), Some(MAIN_TOKEN))
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn oauth_identity_and_restrictions_survive_the_real_rmcp_transport() {
        use crate::tools::OpsBrain;
        use rmcp::transport::streamable_http_server::{
            session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
        };

        // Every tested refusal must happen before touching this lazy pool.
        let pool = sqlx::postgres::PgPoolOptions::new()
            .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let mcp: StreamableHttpService<OpsBrain, LocalSessionManager> = StreamableHttpService::new(
            move || Ok(OpsBrain::new(pool.clone(), None)),
            Default::default(),
            StreamableHttpServerConfig::default()
                .with_json_response(true)
                .with_sse_keep_alive(None),
        );
        let auth = AuthState {
            main_token: Some(MAIN_TOKEN.into()),
            agent_tokens: Arc::new(vec![]),
            machine_tokens: Arc::new(vec![]),
        };
        let app =
            Router::new()
                .nest_service("/mcp", mcp)
                .layer(axum::middleware::from_fn_with_state(
                    OAuthAuthState {
                        auth,
                        oauth: Some(Arc::new(state())),
                    },
                    oauth_bearer_auth,
                ));
        let token = signed(&claims());
        let request = |body: Value, session: Option<&str>| {
            let mut builder = Request::post("/mcp")
                .header(header::HOST, "localhost")
                .header(header::AUTHORIZATION, format!("Bearer {token}"))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::ACCEPT, "application/json, text/event-stream");
            if let Some(session) = session {
                builder = builder
                    .header("mcp-session-id", session)
                    .header("mcp-protocol-version", "2025-11-25");
            }
            builder.body(Body::from(body.to_string())).unwrap()
        };
        let init = app.clone().oneshot(request(json!({"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"oauth-fixture","version":"1"}}}), None)).await.unwrap();
        assert_eq!(init.status(), StatusCode::OK);
        let session = init.headers()["mcp-session-id"]
            .to_str()
            .unwrap()
            .to_string();
        to_bytes(init.into_body(), 64 * 1024).await.unwrap();
        let initialized = app
            .clone()
            .oneshot(request(
                json!({"jsonrpc":"2.0","method":"notifications/initialized"}),
                Some(&session),
            ))
            .await
            .unwrap();
        assert_eq!(initialized.status(), StatusCode::ACCEPTED);
        for (name, arguments, expected) in [
            (
                "create_handoff",
                json!({"from_agent":"Forged-Agent","to_agent":"Other-Bot","title":"must reject","body":"must not reach PostgreSQL"}),
                "does not match your token-bound agent",
            ),
            (
                "check_in",
                json!({"agent_name":"Other-Bot"}),
                "excluded from client data",
            ),
            (
                "add_knowledge",
                json!({"author":"Test-Bot","title":"must reject","content":"must not reach PostgreSQL","category":"gotcha","client_slug":"private-client"}),
                "excluded from client data",
            ),
        ] {
            let response = tokio::time::timeout(Duration::from_secs(2), app.clone().oneshot(request(json!({"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":name,"arguments":arguments}}), Some(&session))))
                .await.expect("guard must reject before any database call").unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            let body = to_bytes(response.into_body(), 64 * 1024).await.unwrap();
            let body = String::from_utf8(body.to_vec()).unwrap();
            assert!(body.contains(expected), "{name}: {body}");
        }
    }
}
