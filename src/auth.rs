use axum::{extract::State, http::StatusCode, middleware::Next, response::Response};

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

/// Axum middleware: validates Bearer token on all non-health requests.
pub async fn bearer_auth(
    State(expected_token): State<Option<String>>,
    request: axum::extract::Request,
    next: Next,
) -> Result<Response, StatusCode> {
    // Health endpoint is always public
    if request.uri().path() == "/health" {
        return Ok(next.run(request).await);
    }

    // If no token configured, allow all requests
    let Some(ref expected) = expected_token else {
        return Ok(next.run(request).await);
    };

    let auth_header = request
        .headers()
        .get("authorization")
        .and_then(|v| v.to_str().ok());

    match auth_header {
        Some(header) if header.starts_with("Bearer ") => {
            if validate_token(&header[7..], expected) {
                Ok(next.run(request).await)
            } else {
                Err(StatusCode::UNAUTHORIZED)
            }
        }
        _ => Err(StatusCode::UNAUTHORIZED),
    }
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
}
