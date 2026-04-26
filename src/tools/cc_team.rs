//! `check_in` — pending-work query for a CC.
//!
//! ops-brain is the team bus, not a brain. Each CC already knows who it is
//! from its own CLAUDE.md. `check_in` exists for one reason: to answer
//! "what's pending for me from the rest of the team?" — open handoffs to
//! my CC, recent notifications, open incidents in my scope. It is
//! **opt-in**, not a mandatory startup ritual. Call it when you want to know
//! what's waiting; otherwise just do the work.
//!
//! `CC_TEAM` is the authoritative table of CC names, the hostname aliases
//! that should normalize to each CC, and each CC's client scope. Identity
//! itself lives in each CC's per-machine CLAUDE.md — this table only
//! exists so cross-CC tooling can speak both forms (CC name and hostname)
//! and converge on a single canonical form (CC name).

use rmcp::model::*;
use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, json_result};
use crate::repo::{client_repo, handoff_repo, incident_repo};

/// The four CCs on the team. `(canonical_cc_name, hostname_aliases, client_slug)`.
/// `client_slug = None` means the CC operates globally / has no single client
/// scope (cloud server, dev workstation).
///
/// Multiple hostnames per CC are accepted as legacy/ergonomic aliases — for
/// example CC-CPA's machine has been called both `CPA-SRV` and `SMYT-SERVER`
/// across docs and history. Any alias normalizes to the canonical CC name.
///
/// To add a fifth CC: append one row here, set the new CC's hostname in its
/// per-machine CLAUDE.md, and (if it owns a client) create the client row.
pub const CC_TEAM: &[(&str, &[&str], Option<&str>)] = &[
    ("CC-Cloud", &["kensai-cloud"], None),
    ("CC-Stealth", &["stealth"], None),
    ("CC-HSR", &["HV-FS0"], Some("hsr")),
    ("CC-CPA", &["SMYT-SERVER", "CPA-SRV"], Some("cpa")),
];

/// Normalize a CC name or hostname (any known alias) to its canonical CC name.
/// Case-insensitive on the input. Returns an error string with the allowlist
/// when the input doesn't match anything.
///
/// Examples — all of these resolve to `"CC-Stealth"`:
/// `"CC-Stealth"`, `"cc-stealth"`, `"CC-STEALTH"`, `"stealth"`, `"STEALTH"`.
///
/// This is the right helper to call at any tool boundary that takes a
/// machine name from a user/CC. Strict CC-name validation (no hostnames, no
/// case folding) is `is_valid_cc_name` instead.
pub(crate) fn normalize_machine_name(input: &str) -> Result<&'static str, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err(format!(
            "Empty CC name. Valid: {}",
            cc_allowlist().join(", ")
        ));
    }

    for (cc_name, hostnames, _) in CC_TEAM {
        if cc_name.eq_ignore_ascii_case(trimmed) {
            return Ok(*cc_name);
        }
        for h in *hostnames {
            if h.eq_ignore_ascii_case(trimmed) {
                return Ok(*cc_name);
            }
        }
    }

    Err(format!(
        "Invalid CC name or hostname: '{}'. Valid CC names: {}",
        input,
        cc_allowlist().join(", ")
    ))
}

/// Return the client slug for a canonical CC name. Returns `None` for CCs
/// with global scope (CC-Cloud, CC-Stealth) and for unknown names.
fn client_slug_for(cc_name: &str) -> Option<&'static str> {
    CC_TEAM
        .iter()
        .find(|(n, _, _)| *n == cc_name)
        .and_then(|(_, _, c)| *c)
}

/// True iff `cc_name` is exactly one of the four canonical CC names
/// (case-sensitive, no hostnames). Used by `add_knowledge` provenance
/// validation where strictness matters — knowledge `author_cc` is immutable
/// after write and we don't want hostname-shaped values landing there.
pub(crate) fn is_valid_cc_name(cc_name: &str) -> bool {
    CC_TEAM.iter().any(|(n, _, _)| *n == cc_name)
}

/// All canonical CC names in `CC_TEAM` declaration order.
pub(crate) fn cc_allowlist() -> Vec<&'static str> {
    CC_TEAM.iter().map(|(n, _, _)| *n).collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckInParams {
    /// Your CC name (canonical: CC-Cloud, CC-Stealth, CC-HSR, CC-CPA).
    /// Hostnames are also accepted and normalized.
    pub my_name: String,
}

pub async fn handle_check_in(brain: &super::OpsBrain, p: CheckInParams) -> CallToolResult {
    let cc_name = match normalize_machine_name(&p.my_name) {
        Ok(n) => n,
        Err(e) => return error_result(&e),
    };
    let client_slug = client_slug_for(cc_name);

    // Open action handoffs targeted at your CC. Stored canonical form is
    // the CC name (post-migration `20260426000002`); hostnames are
    // normalized at write time so this lookup always uses the CC name.
    let action_handoffs = match handoff_repo::list_handoffs(
        &brain.pool,
        Some("pending"),
        Some(cc_name),
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

    // Recent notify-class handoffs targeted at your CC (compact: id/title/from/created_at only).
    // Notifications older than NOTIFY_TTL_DAYS are filtered at the repo level.
    let notify_handoffs = match handoff_repo::list_handoffs(
        &brain.pool,
        Some("pending"),
        Some(cc_name),
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
        assert_eq!(names.len(), 4, "duplicate canonical CC names");

        // Hostname aliases must be unique across all CCs (a hostname can't
        // map to two different CCs).
        let mut seen = std::collections::HashSet::new();
        for (cc, hosts, _) in CC_TEAM {
            for h in *hosts {
                let key = h.to_ascii_lowercase();
                assert!(
                    seen.insert(key),
                    "hostname '{h}' appears twice (in or before {cc})"
                );
            }
        }
    }

    #[test]
    fn normalize_canonical_cc_name() {
        assert_eq!(normalize_machine_name("CC-Stealth").unwrap(), "CC-Stealth");
        assert_eq!(normalize_machine_name("CC-Cloud").unwrap(), "CC-Cloud");
        assert_eq!(normalize_machine_name("CC-HSR").unwrap(), "CC-HSR");
        assert_eq!(normalize_machine_name("CC-CPA").unwrap(), "CC-CPA");
    }

    #[test]
    fn normalize_hostname_alias() {
        assert_eq!(normalize_machine_name("stealth").unwrap(), "CC-Stealth");
        assert_eq!(normalize_machine_name("kensai-cloud").unwrap(), "CC-Cloud");
        assert_eq!(normalize_machine_name("HV-FS0").unwrap(), "CC-HSR");
        assert_eq!(normalize_machine_name("SMYT-SERVER").unwrap(), "CC-CPA");
        // Legacy alias still accepted.
        assert_eq!(normalize_machine_name("CPA-SRV").unwrap(), "CC-CPA");
    }

    #[test]
    fn normalize_is_case_insensitive() {
        assert_eq!(normalize_machine_name("cc-stealth").unwrap(), "CC-Stealth");
        assert_eq!(normalize_machine_name("CC-STEALTH").unwrap(), "CC-Stealth");
        assert_eq!(normalize_machine_name("STEALTH").unwrap(), "CC-Stealth");
        assert_eq!(normalize_machine_name("hv-fs0").unwrap(), "CC-HSR");
        assert_eq!(normalize_machine_name("smyt-server").unwrap(), "CC-CPA");
    }

    #[test]
    fn normalize_trims_whitespace() {
        assert_eq!(
            normalize_machine_name("  CC-Stealth  ").unwrap(),
            "CC-Stealth"
        );
        assert_eq!(normalize_machine_name("\tstealth\n").unwrap(), "CC-Stealth");
    }

    #[test]
    fn normalize_rejects_unknown() {
        let err = normalize_machine_name("CC-NotReal").unwrap_err();
        assert!(err.contains("CC-NotReal"));
        assert!(
            err.contains("CC-Stealth"),
            "allowlist should appear in error"
        );

        let err_empty = normalize_machine_name("").unwrap_err();
        assert!(err_empty.contains("Empty"));

        let err_ws = normalize_machine_name("   ").unwrap_err();
        assert!(err_ws.contains("Empty"));

        assert!(normalize_machine_name("random-host").is_err());
    }

    #[test]
    fn is_valid_cc_name_is_strict() {
        assert!(is_valid_cc_name("CC-Stealth"));
        assert!(is_valid_cc_name("CC-Cloud"));
        assert!(is_valid_cc_name("CC-HSR"));
        assert!(is_valid_cc_name("CC-CPA"));

        // Strict mode does NOT accept the things normalize accepts.
        assert!(!is_valid_cc_name("cc-stealth"), "strict is case-sensitive");
        assert!(!is_valid_cc_name("stealth"), "strict rejects hostnames");
        assert!(!is_valid_cc_name(""));
    }

    #[test]
    fn cc_allowlist_returns_all_four_in_order() {
        let names = cc_allowlist();
        assert_eq!(names, vec!["CC-Cloud", "CC-Stealth", "CC-HSR", "CC-CPA"]);
    }

    #[test]
    fn client_slug_for_canonical_cc() {
        assert_eq!(client_slug_for("CC-HSR"), Some("hsr"));
        assert_eq!(client_slug_for("CC-CPA"), Some("cpa"));
        assert_eq!(client_slug_for("CC-Cloud"), None);
        assert_eq!(client_slug_for("CC-Stealth"), None);
        assert_eq!(client_slug_for("not-a-cc"), None);
        // Strict — case mismatch returns None.
        assert_eq!(client_slug_for("cc-hsr"), None);
    }
}
