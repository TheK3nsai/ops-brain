//! `check_in` — pending-work query for a CC.
//!
//! ops-brain is the team bus, not a brain. Each CC already knows who it is
//! from its own CLAUDE.md. `check_in` exists for one reason: to answer
//! "what's pending for me from the rest of the team?" — open handoffs to
//! my machine, recent notifications, open incidents in my scope. It is
//! **opt-in**, not a mandatory startup ritual. Call it when you want to know
//! what's waiting; otherwise just do the work.
//!
//! `CC_TEAM` is a tiny lookup table that translates `my_name` to a hostname
//! (for handoff filtering) and to a client slug (for incident scoping). It
//! is NOT identity storage — identity lives in each CC's per-machine
//! CLAUDE.md.

use rmcp::model::*;
use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, json_result};
use crate::repo::{client_repo, handoff_repo, incident_repo};

/// The four CCs on the team. `(cc_name, hostname, client_slug)`.
/// `client_slug = None` means the CC operates globally / has no single client
/// scope (cloud server, dev workstation).
///
/// To add a fifth CC: append one row here, set the new CC's hostname in its
/// per-machine CLAUDE.md, and (if it owns a client) create the client row.
pub const CC_TEAM: &[(&str, &str, Option<&str>)] = &[
    ("CC-Cloud", "kensai-cloud", None),
    ("CC-Stealth", "stealth", None),
    ("CC-HSR", "HV-FS0", Some("hsr")),
    ("CC-CPA", "CPA-SRV", Some("cpa")),
];

/// Returns `(hostname, client_slug)` if `cc_name` is in the allowlist.
fn lookup(cc_name: &str) -> Option<(&'static str, Option<&'static str>)> {
    CC_TEAM
        .iter()
        .find(|(n, _, _)| *n == cc_name)
        .map(|(_, h, c)| (*h, *c))
}

fn allowlist_names() -> Vec<&'static str> {
    CC_TEAM.iter().map(|(n, _, _)| *n).collect()
}

/// Returns true if `cc_name` exactly matches one of the four valid CC names.
/// Case-sensitive. Exposed for use by other tool handlers (e.g. knowledge
/// provenance validation in `add_knowledge`) that need the same allowlist
/// gate without duplicating the `CC_TEAM` table.
pub(crate) fn is_valid_cc_name(cc_name: &str) -> bool {
    CC_TEAM.iter().any(|(n, _, _)| *n == cc_name)
}

/// Returns the list of all valid CC names, in `CC_TEAM` declaration order.
/// Use when building user-facing error messages that need to display the
/// full allowlist.
pub(crate) fn cc_allowlist() -> Vec<&'static str> {
    allowlist_names()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckInParams {
    /// Your CC name. Must be one of: CC-Cloud, CC-Stealth, CC-HSR, CC-CPA.
    pub my_name: String,
}

pub async fn handle_check_in(brain: &super::OpsBrain, p: CheckInParams) -> CallToolResult {
    let cc_name = p.my_name.trim();
    let (hostname, client_slug) = match lookup(cc_name) {
        Some(t) => t,
        None => {
            return error_result(&format!(
                "Invalid CC name: '{cc_name}'. Valid: {}",
                allowlist_names().join(", ")
            ))
        }
    };

    // Open action handoffs targeted at your machine.
    let action_handoffs = match handoff_repo::list_handoffs(
        &brain.pool,
        Some("pending"),
        Some(hostname),
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

    // Recent notify-class handoffs targeted at your machine (compact: id/title/from/created_at only).
    // Notifications older than NOTIFY_TTL_DAYS are filtered at the repo level.
    // include_notify is ignored when category is set explicitly — passing false for clarity.
    let notify_handoffs = match handoff_repo::list_handoffs(
        &brain.pool,
        Some("pending"),
        Some(hostname),
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
                "from_machine": h.from_machine,
                "created_at": h.created_at,
            })
        })
        .collect();

    // Open incidents in your scope.
    //
    // CRITICAL: list_open_incidents_for_cc filters to client_id IS NULL when
    // client_id is None — global infra only, NOT every incident across every
    // client. The plain list_incidents would surface hospice and CPA incidents
    // to CC-Cloud / CC-Stealth and bypass the cross-client gate.
    //
    // For client-scoped CCs (HSR, CPA), we explicitly fail if the configured
    // client_slug doesn't resolve to a row. The old behavior (silent fall
    // through to client_id = None → global infra) hid CC_TEAM/seed.sql drift
    // by quietly returning the wrong scope; failing loudly is better.
    let client_id = match client_slug {
        Some(slug) => match client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c.id),
            Ok(None) => {
                return error_result(&format!(
                    "CC team config maps {cc_name} to client_slug='{slug}', but no such client \
                     row exists. Check seed.sql or update CC_TEAM in src/tools/cc_team.rs."
                ))
            }
            Err(e) => return error_result(&format!("Failed to load client {slug}: {e}")),
        },
        None => None,
    };
    let incidents =
        match incident_repo::list_open_incidents_for_cc(&brain.pool, client_id, 20).await {
            Ok(v) => v,
            Err(e) => return error_result(&format!("Failed to load incidents: {e}")),
        };

    // Response shape is intentionally minimal: action handoffs, notify-class
    // handoffs (compact), incidents in scope. No identity echo (the CC already
    // knows its own name and hostname — that's the whole point of v1.5).
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
            "client_slug": client_slug,
        },
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cc_team_has_four_unique_entries() {
        assert_eq!(CC_TEAM.len(), 4);
        let names: std::collections::HashSet<_> = CC_TEAM.iter().map(|(n, _, _)| *n).collect();
        let hosts: std::collections::HashSet<_> = CC_TEAM.iter().map(|(_, h, _)| *h).collect();
        assert_eq!(names.len(), 4, "duplicate CC names");
        assert_eq!(hosts.len(), 4, "duplicate hostnames");
    }

    #[test]
    fn allowlist_round_trip() {
        for (n, h, _) in CC_TEAM {
            assert!(lookup(n).is_some(), "{n} should be in allowlist");
            assert_eq!(lookup(n).map(|(host, _)| host), Some(*h));
        }
    }

    #[test]
    fn allowlist_rejects_unknown() {
        assert!(lookup("CC-NotReal").is_none());
        assert!(lookup("").is_none());
        assert!(
            lookup("cc-stealth").is_none(),
            "names should be case-sensitive"
        );
    }

    #[test]
    fn allowlist_names_returns_all_four() {
        let names = allowlist_names();
        assert_eq!(names.len(), 4);
        assert!(names.contains(&"CC-Cloud"));
        assert!(names.contains(&"CC-Stealth"));
        assert!(names.contains(&"CC-HSR"));
        assert!(names.contains(&"CC-CPA"));
    }
}
