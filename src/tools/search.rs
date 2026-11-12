//! `search_bus`: full-text search over handoffs.
//!
//! The bus holds handoffs and nothing else, so this is how an agent finds
//! prior work: a ranked search over titles and bodies, or the most recent
//! handoffs when no query is given. One body in full is `get_handoff`.

use rmcp::model::*;
use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, json_result, truncate_str};
use crate::models::handoff::Handoff;
use crate::validation::deserialize_flexible_i64;

/// Bytes of body kept in a compact result.
const SNIPPET_BYTES: usize = 200;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchBusParams {
    /// Words to find in handoff titles and bodies; all must match, and when
    /// nothing does, any may. A "quoted phrase", `or` and `-word` (exclude)
    /// work as in a web search and are never loosened. Empty or "*" lists
    /// the most recent handoffs instead.
    pub query: Option<String>,
    /// Max results (default 20, clamped to 1..=200)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
    /// A 200-byte `_snippet` instead of the full body and context (default:
    /// true). Use get_handoff for one handoff in full.
    pub compact: Option<bool>,
}

fn compact_handoff(h: &Handoff) -> serde_json::Value {
    let snippet = if h.body.len() > SNIPPET_BYTES {
        format!("{}...", truncate_str(&h.body, SNIPPET_BYTES))
    } else {
        h.body.clone()
    };
    serde_json::json!({
        "id": h.id,
        "title": h.title,
        "status": h.status,
        "priority": h.priority,
        "category": h.category,
        "from_agent": h.from_agent,
        "to_agent": h.to_agent,
        "in_reply_to": h.in_reply_to,
        "created_at": h.created_at,
        "updated_at": h.updated_at,
        "_snippet": snippet,
    })
}

pub async fn handle_search_bus(
    brain: &super::OpsBrain,
    p: SearchBusParams,
    vis: crate::auth::Visibility<'_>,
) -> CallToolResult {
    let page = crate::pagination::PageRequest::new(p.limit, 20);
    let compact = p.compact.unwrap_or(true);
    let query = p.query.as_deref().map(str::trim).unwrap_or("");

    let found = if query.is_empty() || query == "*" {
        // Browse: newest first, notifications included, nothing aged out.
        crate::repo::handoff_repo::list_recent_handoffs(
            &brain.pool,
            vis.party(),
            page.fetch_limit(),
        )
        .await
    } else {
        crate::repo::handoff_repo::search_handoffs(
            &brain.pool,
            query,
            vis.party(),
            page.fetch_limit(),
        )
        .await
    };
    let mut handoffs = match found {
        Ok(v) => v,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };
    let has_more = page.trim(&mut handoffs);

    let items: Vec<serde_json::Value> = if compact {
        handoffs.iter().map(compact_handoff).collect()
    } else {
        handoffs
            .iter()
            .filter_map(|h| serde_json::to_value(h).ok())
            .collect()
    };
    json_result(&serde_json::json!({
        "handoffs": items,
        "limit": page.limit,
        "limit_clamped": page.limit_clamped,
        "has_more": has_more,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_ignores_parameters_of_the_retired_knowledge_search() {
        // Older clients still send `tables`, `mode` and `client_slug`.
        let params: SearchBusParams = serde_json::from_value(serde_json::json!({
            "query": "backup",
            "tables": ["handoffs"],
            "mode": "hybrid",
            "client_slug": "example",
            "limit": "5"
        }))
        .unwrap();
        assert_eq!(params.query.as_deref(), Some("backup"));
        assert_eq!(params.limit, Some(5));
    }
}
