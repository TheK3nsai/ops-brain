// Phase 2: Bearer token auth middleware for HTTP transport
// For now, this is a placeholder.

pub fn validate_token(token: &str, expected: &str) -> bool {
    // Constant-time comparison to prevent timing attacks
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
