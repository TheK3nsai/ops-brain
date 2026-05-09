//! `check_in` — pending-work query for any agent.
//!
//! ops-brain is the team bus. Each agent already knows who it is from its
//! own local config. `check_in` exists for one reason: to answer "what's
//! pending for me from the rest of the team?" — open handoffs to my
//! `agent_name`, recent notifications, open incidents in my scope. It is
//! **opt-in**, not a mandatory startup ritual. Call it when you want to
//! know what's waiting; otherwise just do the work.
//!
//! v2.0 (agent-agnostic): identity is decoupled from client scope. Pass
//! `agent_name` (free-form slug) for handoff routing, and an optional
//! `client_slug` to scope incidents. Omit `client_slug` to see only
//! global infrastructure incidents (`client_id IS NULL`). The cross-client
//! gate continues to apply upstream where relevant.

use rmcp::model::*;
use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, json_result};
use crate::repo::{client_repo, handoff_repo, incident_repo};
use crate::validation::validate_agent_name;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckInParams {
    /// Your agent identifier (free-form slug, 1–80 chars, [a-zA-Z0-9._-]).
    /// Examples: "CC-Stealth", "CC-Cloud", "codex-hsr", "gemini-stealth".
    /// Used to filter handoffs addressed to you.
    #[serde(alias = "my_name")]
    pub agent_name: String,
    /// Optional client slug to scope incidents (e.g. "hsr", "cpa"). Omit
    /// for global-infrastructure incidents only (`client_id IS NULL`).
    /// Unknown slugs return an error.
    pub client_slug: Option<String>,
}

pub async fn handle_check_in(brain: &super::OpsBrain, p: CheckInParams) -> CallToolResult {
    let agent_name = match validate_agent_name(&p.agent_name) {
        Ok(n) => n.to_string(),
        Err(e) => return error_result(&e),
    };

    // Open action handoffs targeted at this agent. Match is exact on the
    // canonical stored value; v1.x normalized hostname aliases to CC names
    // at write time, so legacy rows continue to be discoverable as long as
    // the caller passes the same canonical name they used at write time.
    let action_handoffs = match handoff_repo::list_handoffs(
        &brain.pool,
        Some("pending"),
        Some(&agent_name),
        None,
        Some("action"),
        false,
        20,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return error_result(&format!("Failed to load action handoffs: {e}")),
    };

    // Recent notify-class handoffs targeted at this agent (compact:
    // id/title/from/created_at only). Older than NOTIFY_TTL_DAYS are
    // filtered at the repo level.
    let notify_handoffs = match handoff_repo::list_handoffs(
        &brain.pool,
        Some("pending"),
        Some(&agent_name),
        None,
        Some("notify"),
        false,
        20,
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return error_result(&format!("Failed to load notify handoffs: {e}")),
    };

    let notify_summary: Vec<serde_json::Value> = notify_handoffs
        .iter()
        .map(|h| {
            serde_json::json!({
                "id": h.id,
                "title": h.title,
                "from_agent": h.from_agent,
                "created_at": h.created_at,
            })
        })
        .collect();

    // Resolve optional client scope. Omitted → global incidents only.
    // Unknown slug → loud error (not a silent fall-through to global).
    let (client_id, client_slug_echo) = match p.client_slug.as_deref().map(str::trim) {
        Some(slug) if !slug.is_empty() => {
            match client_repo::get_client_by_slug(&brain.pool, slug).await {
                Ok(Some(c)) => (Some(c.id), Some(slug.to_string())),
                Ok(None) => {
                    return error_result(&format!(
                        "Unknown client_slug '{slug}'. Pass an existing slug or omit \
                         the parameter to see global-infrastructure incidents only."
                    ));
                }
                Err(e) => return error_result(&format!("Failed to load client {slug}: {e}")),
            }
        }
        _ => (None, None),
    };

    let incidents =
        match incident_repo::list_open_incidents_in_scope(&brain.pool, client_id, 20).await {
            Ok(v) => v,
            Err(e) => return error_result(&format!("Failed to load incidents: {e}")),
        };

    json_result(&serde_json::json!({
        "open_handoffs_to_you": {
            "count": action_handoffs.len(),
            "items": action_handoffs,
        },
        "recent_notifications": {
            "count": notify_summary.len(),
            "items": notify_summary,
        },
        "open_incidents_in_your_scope": {
            "count": incidents.len(),
            "items": incidents,
            "client_slug": client_slug_echo,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn check_in_accepts_legacy_my_name_alias() {
        let params: CheckInParams =
            serde_json::from_value(serde_json::json!({"my_name": "CC-Stealth"})).unwrap();
        assert_eq!(params.agent_name, "CC-Stealth");
        assert_eq!(params.client_slug, None);
    }
}
