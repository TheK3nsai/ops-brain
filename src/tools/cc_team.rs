//! CC team self-identity & check-in.
//!
//! Identity is self-declared by each CC on its first call of every session
//! (`check_in`), self-authored via `set_my_identity`, and surfaced back to the
//! whole team in the briefing returned by `check_in`. Per-session identity
//! lives in `OpsBrain.cc_name` — `StreamableHttpService` constructs a fresh
//! `OpsBrain` per session, so it's per-CC state without any request-context
//! plumbing through the MCP transport layer.
//!
//! ## Adding a fifth CC
//! Append one row to `CC_TEAM`, update each CC's per-machine CLAUDE.md to tell
//! that CC its own name, and (if it owns a client) create the client row first.
//! The migration's TEXT PK accepts anything; no schema change needed.

use rmcp::model::*;
use schemars::JsonSchema;
use serde::Deserialize;

use super::helpers::{error_result, json_result};
use crate::repo::{cc_identity_repo, client_repo, handoff_repo, incident_repo};

/// The four CCs on the team. `(cc_name, hostname, client_slug)`.
/// `client_slug = None` means the CC operates globally / has no single client
/// scope (cloud server, dev workstation).
pub const CC_TEAM: &[(&str, &str, Option<&str>)] = &[
    ("CC-Cloud", "kensai-cloud", None),
    ("CC-Stealth", "stealth", None),
    ("CC-HSR", "HV-FS0", Some("hsr")),
    ("CC-CPA", "CPA-SRV", Some("cpa")),
];

const MIN_BODY_CHARS: usize = 20;
const MAX_BODY_CHARS: usize = 2000;

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

// ===== check_in =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CheckInParams {
    /// Your CC name on the team. Must be one of: CC-Cloud, CC-Stealth,
    /// CC-HSR, CC-CPA. Each CC's per-machine CLAUDE.md tells it its own name.
    pub my_name: String,
}

pub async fn handle_check_in(brain: &super::OpsBrain, p: CheckInParams) -> CallToolResult {
    let cc_name = p.my_name.trim();
    if lookup(cc_name).is_none() {
        return error_result(&format!(
            "Invalid CC name: '{cc_name}'. Valid: {}",
            allowlist_names().join(", ")
        ));
    }

    // Set per-session identity. The Arc<RwLock> is per-session because
    // StreamableHttpService creates a fresh OpsBrain per session.
    //
    // Same-name re-checks are idempotent (the tool description advertises
    // refresh-via-recheck). Re-checking with a DIFFERENT name is rejected:
    // this guarantees that "a CC can only ever update its own row" is
    // literally true within a session, and catches the (unlikely) bug where
    // a confused CC tries to impersonate another.
    {
        let mut guard = brain.cc_name.write().await;
        if let Some(prev) = guard.as_deref() {
            if prev != cc_name {
                return error_result(&format!(
                    "Already checked in as {prev}. Cannot switch identity to {cc_name} \
                     mid-session — start a new session to change CC name."
                ));
            }
        }
        *guard = Some(cc_name.to_string());
    }

    match build_briefing(brain, cc_name).await {
        Ok(payload) => json_result(&payload),
        Err(msg) => error_result(&msg),
    }
}

// ===== set_my_identity =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SetMyIdentityParams {
    /// Your own confident description of who you are and what you own.
    /// Markdown supported. Your peers see this on every check_in.
    /// 20-2000 characters.
    pub body: String,
}

pub async fn handle_set_my_identity(
    brain: &super::OpsBrain,
    p: SetMyIdentityParams,
) -> CallToolResult {
    let cc_name = match brain.cc_name.read().await.clone() {
        Some(n) => n,
        None => {
            return error_result(
                "You haven't checked in yet. Call `check_in` first with your CC name.",
            )
        }
    };

    let body = p.body.trim();
    let char_count = body.chars().count();
    if char_count < MIN_BODY_CHARS {
        return error_result(&format!(
            "Identity body too short ({char_count} chars). Minimum {MIN_BODY_CHARS}. \
             Write a real, confident description of your scope — your peers will read this."
        ));
    }
    if char_count > MAX_BODY_CHARS {
        return error_result(&format!(
            "Identity body too long ({char_count} chars). Maximum {MAX_BODY_CHARS}."
        ));
    }

    let (row, was_first_write) = match cc_identity_repo::upsert(&brain.pool, &cc_name, body).await {
        Ok(r) => r,
        Err(e) => return error_result(&format!("Failed to write identity for {cc_name}: {e}")),
    };

    let announced_to: Vec<String> = if was_first_write {
        announce_introduction(brain, &cc_name, body).await
    } else {
        Vec::new()
    };

    json_result(&serde_json::json!({
        "ok": true,
        "cc_name": row.cc_name,
        "updated_at": row.updated_at,
        "first_write": was_first_write,
        "announced_to": announced_to,
        "next": "Call check_in next time you start a session — your scope is now part of the team briefing.",
    }))
}

// ===== Briefing assembly =====

async fn build_briefing(
    brain: &super::OpsBrain,
    cc_name: &str,
) -> Result<serde_json::Value, String> {
    let (hostname, client_slug) =
        lookup(cc_name).ok_or_else(|| format!("Unknown CC: {cc_name}"))?;

    // Self-authored scope (or bootstrap message).
    let identity = cc_identity_repo::get(&brain.pool, cc_name)
        .await
        .map_err(|e| format!("Failed to load identity for {cc_name}: {e}"))?;
    let (your_scope, scope_status) = match identity {
        Some(i) => (i.body, "self_authored"),
        None => (bootstrap_message(cc_name, hostname), "bootstrap"),
    };

    // Team roster — every other CC's self-authored line, or null if not yet written.
    let all = cc_identity_repo::list_all(&brain.pool)
        .await
        .map_err(|e| format!("Failed to load team roster: {e}"))?;
    let by_name: std::collections::HashMap<String, String> =
        all.into_iter().map(|i| (i.cc_name, i.body)).collect();
    let team: Vec<serde_json::Value> = CC_TEAM
        .iter()
        .filter(|(n, _, _)| *n != cc_name)
        .map(|(n, h, _)| {
            let scope = by_name.get(*n).cloned();
            let status = if scope.is_some() {
                "self_authored"
            } else {
                "not_yet_written"
            };
            serde_json::json!({
                "cc_name": n,
                "hostname": h,
                "scope": scope,
                "status": status,
            })
        })
        .collect();

    // Open handoffs targeted at your machine.
    let handoffs =
        handoff_repo::list_handoffs(&brain.pool, Some("pending"), Some(hostname), None, 20)
            .await
            .map_err(|e| format!("Failed to load handoffs: {e}"))?;

    // Open incidents in your scope (client-scoped for CC-HSR/CC-CPA, global otherwise).
    let client_id = match client_slug {
        Some(slug) => client_repo::get_client_by_slug(&brain.pool, slug)
            .await
            .map_err(|e| format!("Failed to load client {slug}: {e}"))?
            .map(|c| c.id),
        None => None,
    };
    // CRITICAL: use list_open_incidents_for_cc — when client_id is None it
    // filters to client_id IS NULL (global infra only), NOT every incident
    // across every client. The plain list_incidents would surface hospice and
    // CPA incidents to CC-Cloud / CC-Stealth and bypass the cross-client gate.
    let incidents = incident_repo::list_open_incidents_for_cc(&brain.pool, client_id, 20)
        .await
        .map_err(|e| format!("Failed to load incidents for {cc_name}: {e}"))?;

    Ok(serde_json::json!({
        "you": cc_name,
        "hostname": hostname,
        "your_scope": your_scope,
        "your_scope_status": scope_status,
        "team": team,
        "open_handoffs_to_you": {
            "count": handoffs.len(),
            "items": handoffs,
        },
        "open_incidents_in_your_scope": {
            "count": incidents.len(),
            "items": incidents,
            "client_slug": client_slug,
        },
        "next": next_steps_hint(scope_status),
    }))
}

fn next_steps_hint(status: &str) -> &'static str {
    match status {
        "bootstrap" => {
            "Your scope is empty. Read your_scope, then call set_my_identity to write your own."
        }
        _ => {
            "Handle the user's task first. Process open handoffs and incidents at a natural pause."
        }
    }
}

fn bootstrap_message(cc_name: &str, hostname: &str) -> String {
    format!(
        "Welcome, {cc_name}. You haven't written your scope yet — and no one is going to write it for you.\n\n\
         Take a few minutes with `get_situational_awareness` to look around what you own from {hostname}: your servers, your services, your open handoffs, your incidents. Then call `set_my_identity` with your own confident description of who you are, what you protect, and what your teammates can count on you for.\n\n\
         Make it yours. Your peers will read this every time they check in. You're the expert on this scope — act like it."
    )
}

// ===== Announcement handoffs (first-write sweetener) =====

/// Fan out a low-priority introduction handoff to every other CC's machine.
/// Best-effort: per-peer failures are logged but don't fail the parent call.
async fn announce_introduction(brain: &super::OpsBrain, new_cc: &str, body: &str) -> Vec<String> {
    let from_hostname = match lookup(new_cc) {
        Some((h, _)) => h,
        None => return Vec::new(),
    };

    let snippet = first_sentence(body, 200);
    let title = format!("{new_cc} has introduced themselves");
    let announce_body = format!(
        "{new_cc} just wrote their team identity for the first time:\n\n> {snippet}\n\n\
         Call `check_in` to see the full team roster."
    );

    let mut announced = Vec::new();
    for (peer_name, peer_host, _) in CC_TEAM.iter() {
        if *peer_name == new_cc {
            continue;
        }
        match handoff_repo::create_handoff(
            &brain.pool,
            None,
            from_hostname,
            Some(peer_host),
            "low",
            &title,
            &announce_body,
            None,
        )
        .await
        {
            Ok(_) => announced.push((*peer_name).to_string()),
            Err(e) => {
                tracing::warn!("Failed to announce {new_cc} introduction to {peer_name}: {e}")
            }
        }
    }
    announced
}

/// Extract the first sentence (up to `max_chars`) from a body. Stops at the
/// first '.', '!', '?', or newline; falls back to truncation at `max_chars`.
fn first_sentence(body: &str, max_chars: usize) -> String {
    let trimmed = body.trim();
    let stop = trimmed
        .char_indices()
        .find(|(_, c)| matches!(c, '.' | '!' | '?' | '\n'))
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(trimmed.len());
    let max_byte = byte_index_at_char(trimmed, max_chars);
    let cut = stop.min(max_byte);
    trimmed[..cut].trim().to_string()
}

fn byte_index_at_char(s: &str, char_count: usize) -> usize {
    s.char_indices()
        .nth(char_count)
        .map(|(i, _)| i)
        .unwrap_or(s.len())
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
    fn first_sentence_basic() {
        assert_eq!(first_sentence("Hello there. More.", 200), "Hello there.");
        assert_eq!(first_sentence("One sentence", 200), "One sentence");
        assert_eq!(first_sentence("Line one\nLine two", 200), "Line one");
        assert_eq!(first_sentence("?", 200), "?");
        assert_eq!(first_sentence("  trimmed.  ", 200), "trimmed.");
    }

    #[test]
    fn first_sentence_truncates_at_max_chars() {
        let long = "abcdefghij".repeat(50); // 500 chars, no terminator
        let cut = first_sentence(&long, 50);
        assert!(cut.chars().count() <= 50);
    }

    #[test]
    fn first_sentence_handles_multibyte() {
        // Stop char (period) is ASCII, but body has multibyte chars before it.
        assert_eq!(first_sentence("Olá mundo. extra", 200), "Olá mundo.");
    }

    #[test]
    fn next_steps_hint_branches() {
        assert!(next_steps_hint("bootstrap").contains("set_my_identity"));
        assert!(next_steps_hint("self_authored").contains("user's task"));
    }

    #[test]
    fn bootstrap_message_personalized() {
        let msg = bootstrap_message("CC-Stealth", "stealth");
        assert!(msg.contains("CC-Stealth"));
        assert!(msg.contains("stealth"));
        assert!(msg.contains("set_my_identity"));
        assert!(msg.contains("get_situational_awareness"));
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
