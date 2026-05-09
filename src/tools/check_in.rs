//! `check_in` — pending-work query for any agent.
//!
//! ops-brain is the team bus. Each agent already knows who it is from its
//! own local config. `check_in` exists for one reason: to answer "what's
//! pending for me from the rest of the team?" — open action handoffs to
//! my `agent_name`, plus recent notify-class handoffs. It is **opt-in**,
//! not a mandatory startup ritual. Call it when you want to know what's
//! waiting; otherwise just do the work.
//!
//! v2.0 (agent-agnostic): identity is a free-form slug.
//! v3.0 (de-bloat): incidents subsystem removed; check_in only returns handoffs.

use rmcp::model::*;
use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, json_result};
use crate::repo::handoff_repo;
use crate::validation::validate_agent_name;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckInParams {
    /// Your agent identifier (free-form slug, 1–80 chars, [a-zA-Z0-9._-]).
    /// Examples: "CC-Stealth", "CC-Cloud", "codex-hsr", "gemini-stealth".
    /// Used to filter handoffs addressed to you.
    #[serde(alias = "my_name")]
    pub agent_name: String,
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

    json_result(&serde_json::json!({
        "open_handoffs_to_you": {
            "count": action_handoffs.len(),
            "items": action_handoffs,
        },
        "recent_notifications": {
            "count": notify_summary.len(),
            "items": notify_summary,
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
    }
}
