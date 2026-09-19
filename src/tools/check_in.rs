//! `check_in` — pending-work query for any agent.
//!
//! ops-brain is the team bus. Each agent already knows who it is from its
//! own local config. `check_in` exists for one reason: to answer "what's
//! pending for me from the rest of the team?" — open action handoffs to
//! my `agent_name`, plus recent notify-class handoffs. It is **opt-in**,
//! not a mandatory startup ritual. Call it when you want to know what's
//! waiting; otherwise just do the work.
//!
//! Both sections are capped pages. They fetch one row past the cap and report
//! `has_more`, so "20 action handoffs" never silently means "20 of 40" —
//! list_handoffs serves the rest.

use rmcp::model::*;
use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, json_result};
use crate::pagination::PageRequest;
use crate::repo::handoff_repo;
use crate::validation::validate_agent_name;

const ACTION_LIMIT: i64 = 20;
const NOTIFICATION_LIMIT: i64 = 5;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckInParams {
    /// Your agent identifier (free-form slug, 1–80 chars, [a-zA-Z0-9._-]).
    /// Examples: "CC-Stealth", "CC-Cloud", "Codex-HSR".
    /// Used to filter handoffs addressed to you.
    #[serde(alias = "my_name")]
    pub agent_name: String,
}

pub async fn handle_check_in(
    brain: &super::OpsBrain,
    p: CheckInParams,
    bound: Option<&str>,
) -> CallToolResult {
    let agent_name = match validate_agent_name(&p.agent_name) {
        Ok(n) => n.to_string(),
        Err(e) => return error_result(&e),
    };
    // Read path: checking in as another agent is legitimate (e.g. an
    // interactive session triaging a peer's queue) but worth surfacing.
    crate::auth::warn_identity_mismatch(bound, &agent_name, "check_in");

    // Open action handoffs targeted at this agent, plus unaddressed ones from
    // anyone else — `create_handoff` advertises an omitted `to_agent` as "any
    // agent can pick it up", and check_in is where that promise is kept.
    // Match is exact (case-insensitive) on the canonical stored value; v1.x
    // normalized hostname aliases to CC names at write time, so legacy rows
    // continue to be discoverable as long as the caller passes the same
    // canonical name they used at write time.
    let action_page = PageRequest::new(None, ACTION_LIMIT);
    let mut action_handoffs = match handoff_repo::list_open_handoffs(
        &brain.pool,
        Some(&agent_name),
        None,
        Some("action"),
        /* include_notify */ false,
        /* include_broadcast */ true,
        /* exclude_self_claims */ true,
        action_page.fetch_limit(),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return error_result(&format!("Failed to load action handoffs: {e}")),
    };
    let action_has_more = action_page.trim(&mut action_handoffs);

    // Accepted handoffs this agent filed to itself are lock records, not
    // inbound work: counted here, listed by list_handoffs. The count is
    // decoration — if it fails, report null ("unknown") rather than lose the
    // action page or claim a false zero.
    let self_claims = match handoff_repo::count_self_claims(&brain.pool, &agent_name).await {
        Ok(c) => serde_json::json!({
            "count": c.count,
            "oldest_age_days": c.oldest_age_days,
        }),
        Err(e) => {
            tracing::warn!("check_in: self-claim count failed: {e}");
            serde_json::Value::Null
        }
    };

    // Recent notify-class handoffs targeted at this agent or broadcast
    // (compact: id/title/from/created_at only). Older than NOTIFY_TTL_DAYS are
    // filtered at the repo level.
    let notify_page = PageRequest::new(None, NOTIFICATION_LIMIT);
    let mut notify_handoffs = match handoff_repo::list_open_handoffs(
        &brain.pool,
        Some(&agent_name),
        None,
        Some("notify"),
        /* include_notify */ false,
        /* include_broadcast */ true,
        /* exclude_self_claims */ false,
        notify_page.fetch_limit(),
    )
    .await
    {
        Ok(v) => v,
        Err(e) => return error_result(&format!("Failed to load notify handoffs: {e}")),
    };
    let notify_has_more = notify_page.trim(&mut notify_handoffs);

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

    // `count` is the page count, not the total: `has_more` says whether older
    // rows were left behind for list_handoffs.
    json_result(&serde_json::json!({
        "open_handoffs_to_you": {
            "count": action_handoffs.len(),
            "pending_count": action_handoffs.iter().filter(|h| h.status == "pending").count(),
            "accepted_count": action_handoffs.iter().filter(|h| h.status == "accepted").count(),
            "has_more": action_has_more,
            "self_claims_held": self_claims,
            "items": action_handoffs,
        },
        "recent_notifications": {
            "count": notify_summary.len(),
            "has_more": notify_has_more,
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
