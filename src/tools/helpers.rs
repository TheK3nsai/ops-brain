use rmcp::model::*;
use serde::Serialize;
use std::collections::HashMap;

/// Helper to format tool results as JSON text content.
pub(crate) fn json_result<T: Serialize>(data: &T) -> CallToolResult {
    match serde_json::to_string_pretty(data) {
        Ok(json) => CallToolResult::success(vec![Content::text(json)]),
        Err(e) => CallToolResult::error(vec![Content::text(format!("Serialization error: {e}"))]),
    }
}

pub(crate) fn error_result(msg: &str) -> CallToolResult {
    CallToolResult::error(vec![Content::text(msg.to_string())])
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

/// Truncate `s` to at most `max_bytes`, walking back to the nearest UTF-8 char
/// boundary so the result is always valid UTF-8. No suffix is appended — each
/// caller owns its own ellipsis/format. If `s` already fits, it is returned
/// unchanged. This is the single boundary-walk used by the compact-mode body
/// and snippet truncation across coordination.rs and knowledge.rs.
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
    CallToolResult::error(vec![Content::text(format!("{entity} not found: {key}"))])
}

/// Like `not_found`, but queries pg_trgm for similar slugs and appends "Did you mean: ..." suggestions.
pub(crate) async fn not_found_with_suggestions(
    pool: &sqlx::PgPool,
    entity: &str,
    key: &str,
) -> CallToolResult {
    let table = match entity {
        "Client" => "clients",
        _ => return not_found(entity, key),
    };
    let suggestions = crate::repo::suggest_repo::suggest_similar_slugs(pool, table, key).await;
    if suggestions.is_empty() {
        not_found(entity, key)
    } else {
        CallToolResult::error(vec![Content::text(format!(
            "{entity} not found: {key}. Did you mean: {}?",
            suggestions.join(", ")
        ))])
    }
}

/// Resolve an optional client slug to its id.
///
/// - `Ok(None)` when `slug` is `None` — no client scope requested.
/// - `Ok(Some(id))` on a hit.
/// - `Err(CallToolResult)` on a miss (slug suggestions attached) or a DB
///   error. The `Err` value is a ready-to-return tool error, so call sites
///   collapse the old five-line resolution block to:
///   `match resolve_client_id(pool, slug).await { Ok(v) => v, Err(r) => return r }`.
pub(crate) async fn resolve_client_id(
    pool: &sqlx::PgPool,
    slug: Option<&str>,
) -> Result<Option<uuid::Uuid>, CallToolResult> {
    match slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(pool, slug).await {
            Ok(Some(c)) => Ok(Some(c.id)),
            Ok(None) => Err(not_found_with_suggestions(pool, "Client", slug).await),
            Err(e) => Err(error_result(&format!("Database error: {e}"))),
        },
        None => Ok(None),
    }
}

/// Result of cross-client scope filtering.
pub(crate) struct CrossClientFilterResult {
    /// Items that passed the gate (with _provenance fields injected)
    pub allowed: Vec<serde_json::Value>,
    /// Grouped notices about withheld content (for response)
    pub withheld_notices: Vec<serde_json::Value>,
    /// Individual (entity_id, owning_client_id, action) for audit logging
    pub audit_entries: Vec<(uuid::Uuid, Option<uuid::Uuid>, String)>,
}

/// Partition items into allowed and withheld based on cross-client scope.
///
/// Rules:
/// - No requesting client → all items allowed (no scope to enforce)
/// - Item client_id is NULL → allowed (global content)
/// - Item client_id == requesting → allowed (same client)
/// - Item client_id != requesting + cross_client_safe=true → allowed
/// - Item client_id != requesting + cross_client_safe=false + acknowledge=true → allowed (released)
/// - Item client_id != requesting + cross_client_safe=false + acknowledge=false → WITHHELD
pub(crate) fn filter_cross_client(
    items: Vec<serde_json::Value>,
    entity_type: &str,
    requesting_client_id: Option<uuid::Uuid>,
    acknowledge: bool,
    client_lookup: &HashMap<uuid::Uuid, (String, String)>,
) -> CrossClientFilterResult {
    let Some(req_cid) = requesting_client_id else {
        // No requesting client scope — all items are allowed, inject provenance
        let allowed = items
            .into_iter()
            .map(|mut item| {
                inject_provenance(&mut item, client_lookup);
                item
            })
            .collect();
        return CrossClientFilterResult {
            allowed,
            withheld_notices: Vec::new(),
            audit_entries: Vec::new(),
        };
    };

    let mut allowed = Vec::new();
    let mut withheld_by_client: HashMap<uuid::Uuid, Vec<uuid::Uuid>> = HashMap::new();
    let mut audit_entries = Vec::new();

    for mut item in items {
        let item_client_id = item
            .get("client_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        let cross_client_safe = item
            .get("cross_client_safe")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let entity_id = item
            .get("id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok());

        match item_client_id {
            // Global content (no client_id) — always allowed
            None => {
                inject_provenance(&mut item, client_lookup);
                allowed.push(item);
            }
            // Same client — allowed
            Some(cid) if cid == req_cid => {
                inject_provenance(&mut item, client_lookup);
                allowed.push(item);
            }
            // Different client but marked safe — allowed
            Some(cid) if cross_client_safe => {
                inject_provenance(&mut item, client_lookup);
                allowed.push(item);
                if let Some(eid) = entity_id {
                    audit_entries.push((eid, Some(cid), "released_safe".to_string()));
                }
            }
            // Different client, not safe, but acknowledged — released
            Some(cid) if acknowledge => {
                inject_provenance(&mut item, client_lookup);
                allowed.push(item);
                if let Some(eid) = entity_id {
                    audit_entries.push((eid, Some(cid), "released".to_string()));
                }
            }
            // Different client, not safe, not acknowledged — WITHHELD
            Some(cid) => {
                if let Some(eid) = entity_id {
                    withheld_by_client.entry(cid).or_default().push(eid);
                    audit_entries.push((eid, Some(cid), "withheld".to_string()));
                }
            }
        }
    }

    // Build grouped withheld notices
    let withheld_notices: Vec<serde_json::Value> = withheld_by_client
        .into_iter()
        .map(|(cid, entity_ids)| {
            let (slug, name) = client_lookup
                .get(&cid)
                .cloned()
                .unwrap_or_else(|| ("unknown".to_string(), "Unknown".to_string()));
            serde_json::json!({
                "entity_type": entity_type,
                "count": entity_ids.len(),
                "owning_client_slug": slug,
                "owning_client_name": name,
                "message": format!(
                    "{} {}(s) from client '{}' withheld — cross-client scope mismatch. Re-call with acknowledge_cross_client: true to release.",
                    entity_ids.len(), entity_type, name
                )
            })
        })
        .collect();

    CrossClientFilterResult {
        allowed,
        withheld_notices,
        audit_entries,
    }
}

/// Inject _provenance fields (client slug + name) into a JSON item.
pub(crate) fn inject_provenance(
    item: &mut serde_json::Value,
    client_lookup: &HashMap<uuid::Uuid, (String, String)>,
) {
    if let Some(obj) = item.as_object_mut() {
        let client_id = obj
            .get("client_id")
            .and_then(|v| v.as_str())
            .and_then(|s| uuid::Uuid::parse_str(s).ok());
        match client_id {
            Some(cid) => {
                if let Some((slug, name)) = client_lookup.get(&cid) {
                    obj.insert(
                        "_client_slug".to_string(),
                        serde_json::Value::String(slug.clone()),
                    );
                    obj.insert(
                        "_client_name".to_string(),
                        serde_json::Value::String(name.clone()),
                    );
                }
            }
            None => {
                obj.insert("_client_slug".to_string(), serde_json::Value::Null);
                obj.insert(
                    "_client_name".to_string(),
                    serde_json::Value::String("Global".to_string()),
                );
            }
        }
    }
}
