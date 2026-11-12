use rmcp::model::*;
use serde::Serialize;

/// Return protocol-native structured JSON. rmcp also mirrors the compact JSON
/// into text content for clients that have not adopted `structuredContent`.
pub(crate) fn json_result<T: Serialize>(data: &T) -> CallToolResult {
    match serde_json::to_value(data) {
        Ok(json @ serde_json::Value::Object(_)) => CallToolResult::structured(json),
        Ok(json) => CallToolResult::structured(serde_json::json!({ "result": json })),
        Err(e) => CallToolResult::error(vec![ContentBlock::text(format!(
            "Serialization error: {e}"
        ))]),
    }
}

pub(crate) fn error_result(msg: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(msg.to_string())])
}

/// Resolve the caller's server-bound agent identity from the MCP tool-call
/// context, if they authenticated with a per-agent token.
///
/// The chain: `bearer_auth` inserts a [`crate::auth::CallerClass`] into the
/// HTTP request extensions; rmcp's streamable-HTTP transport injects that
/// request's [`http::request::Parts`] (extensions included) into the tool-call
/// context. So the identity travels transport → middleware → tool with no
/// schema change. Returns `None` when the caller is unbound — the main bearer,
/// a machine caller (which never reaches `/mcp`), or the stdio transport (no
/// HTTP parts at all). `None` means "no identity enforcement", which is correct
/// for every context in which it can occur: main bearer is operator
/// break-glass, and stdio/dev are trusted-local.
///
/// Note `axum::http::request::Parts` is the exact type rmcp inserts — the crate
/// graph resolves `http` to a single version, so the type identity holds and
/// the downcast actually fires (a version split would silently return `None`
/// and disable enforcement).
pub(crate) fn bound_agent(ext: &Extensions) -> Option<String> {
    ext.get::<axum::http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<crate::auth::CallerClass>())
        .and_then(|caller| caller.bound_agent().map(str::to_string))
}

/// The caller's bound slug when its token sets `exclude_client_data`, resolved
/// through the same transport chain as [`bound_agent`]. Handlers turn it into a
/// [`crate::auth::Visibility`]. `None` means unrestricted.
pub(crate) fn restricted_agent(ext: &Extensions) -> Option<String> {
    ext.get::<axum::http::request::Parts>()
        .and_then(|parts| parts.extensions.get::<crate::auth::CallerClass>())
        .and_then(|caller| caller.restricted_agent().map(str::to_string))
}

/// Truncate `s` to at most `max_bytes`, walking back to the nearest UTF-8 char
/// boundary so the result is always valid UTF-8. No suffix is appended — each
/// caller owns its own ellipsis/format. If `s` already fits, it is returned
/// unchanged. This is the single boundary-walk used by the compact-mode body
/// and snippet truncation in coordination.rs and search.rs.
pub(crate) fn truncate_str(s: &str, max_bytes: usize) -> String {
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

pub(crate) fn not_found(entity: &str, key: &str) -> CallToolResult {
    CallToolResult::error(vec![ContentBlock::text(format!(
        "{entity} not found: {key}"
    ))])
}
