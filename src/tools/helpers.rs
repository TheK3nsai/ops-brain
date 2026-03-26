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

pub(crate) fn not_found(entity: &str, key: &str) -> CallToolResult {
    CallToolResult::error(vec![Content::text(format!("{entity} not found: {key}"))])
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

/// Fields to keep per entity type in compact mode. Everything else is stripped.
pub(crate) fn compact_keep_fields(entity_type: &str) -> &'static [&'static str] {
    match entity_type {
        "server" => &[
            "id",
            "hostname",
            "slug",
            "os",
            "ip_address",
            "status",
            "roles",
            "site_id",
        ],
        "site" => &["id", "name", "slug", "address", "client_id"],
        "client" => &["id", "name", "slug"],
        "service" => &["id", "name", "slug", "port", "protocol", "criticality"],
        "network" => &["id", "name", "cidr", "vlan_id"],
        "vendor" => &["id", "name", "category"],
        "incident" => &[
            "id",
            "title",
            "severity",
            "status",
            "client_id",
            "reported_at",
            "resolved_at",
            "time_to_resolve_minutes",
            "cross_client_safe",
            "_client_slug",
            "_client_name",
        ],
        "runbook" => &[
            "id",
            "title",
            "slug",
            "category",
            "client_id",
            "cross_client_safe",
            "_client_slug",
            "_client_name",
        ],
        "handoff" => &[
            "id",
            "title",
            "status",
            "priority",
            "from_machine",
            "to_machine",
            "created_at",
        ],
        "knowledge" => &[
            "id",
            "title",
            "category",
            "client_id",
            "cross_client_safe",
            "_client_slug",
            "_client_name",
        ],
        "monitor" => &["name", "status_text", "monitor_type"],
        "ticket" => &["ticket_id", "title", "state", "priority", "created_at"],
        _ => &["id", "title", "slug", "name"],
    }
}

/// Strip a JSON value down to only the fields allowed for its entity type.
pub(crate) fn compact_value(val: &serde_json::Value, entity_type: &str) -> serde_json::Value {
    let Some(obj) = val.as_object() else {
        return val.clone();
    };
    let keep = compact_keep_fields(entity_type);
    let compacted: serde_json::Map<String, serde_json::Value> = obj
        .iter()
        .filter(|(k, _)| keep.contains(&k.as_str()))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    serde_json::Value::Object(compacted)
}

/// Apply compact mode to a Vec of JSON values.
pub(crate) fn compact_vec(
    items: &[serde_json::Value],
    entity_type: &str,
) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|v| compact_value(v, entity_type))
        .collect()
}

/// Check if a section is included (None means all sections included).
pub(crate) fn section_included(sections: &Option<Vec<String>>, name: &str) -> bool {
    match sections {
        None => true,
        Some(list) => list.iter().any(|s| s == name),
    }
}
