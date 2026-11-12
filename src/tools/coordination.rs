use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{error_result, json_result, not_found, truncate_str};
use super::shared::embed_and_store;
use rmcp::model::*;

// ===== HANDOFF PARAMS =====

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CreateHandoffParams {
    /// Sender agent (free-form slug, 1–80 chars, [a-zA-Z0-9._-]).
    /// Examples: "Claude-Laptop", "Codex-Buildbox".
    #[serde(alias = "from_machine")]
    pub from_agent: String,
    /// Target agent (free-form slug, same form as `from_agent`). Required for
    /// `action`. Omit only to broadcast a `notify`, which then shows in every
    /// other agent's `check_in` for 7 days.
    #[serde(alias = "to_machine")]
    pub to_agent: Option<String>,
    /// Priority: low, normal, high, or critical
    pub priority: Option<String>,
    /// Category: "action" (default — persistent until completed) or "notify"
    /// (ephemeral FYI, auto-pruned from operational queries after 7 days).
    /// Use "notify" for introductions, "I just shipped X" announcements —
    /// anything the recipient doesn't need to act on.
    pub category: Option<String>,
    /// Short title for the handoff (max 200 bytes)
    pub title: String,
    /// Detailed body, markdown supported (max 100,000 bytes)
    pub body: String,
    /// Optional structured context: a JSON object, max 8,192 bytes serialized.
    /// Point at evidence (paths, IDs, URLs) instead of inlining it.
    pub context: Option<serde_json::Value>,
    /// Parent handoff ID (full UUID) when this handoff is a reply. A reply is
    /// `action` when it needs a response and `notify` when it only reports.
    pub in_reply_to: Option<String>,
    /// With `in_reply_to`: also complete the parent in the same call. Use it
    /// when this reply reports the work the parent asked you for. The parent
    /// closes only if it is open and addressed to (or accepted by) you;
    /// otherwise the reply is still filed and `_warning` says why not.
    pub complete_parent: Option<bool>,
    /// With `complete_parent`: commit ref (typically a git SHA) of the work
    /// that closed the parent, recorded on the parent.
    pub commit_hash: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetHandoffParams {
    /// Full handoff UUID.
    pub handoff_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateHandoffStatusParams {
    /// Handoff ID (UUID)
    pub handoff_id: String,
    /// Concrete acceptor identity. Required for family credentials. Fixed
    /// agent tokens record their bound identity; a supplied name must match
    /// the fixed identity or family.
    pub agent_name: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct CompleteHandoffParams {
    /// Handoff ID (UUID)
    pub handoff_id: String,
    /// Optional commit ref (typically a git SHA) recording the work that
    /// closed this handoff.
    pub commit_hash: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListHandoffsParams {
    /// Filter by status: pending, accepted, or completed (`merged` matches
    /// legacy rows only)
    pub status: Option<String>,
    /// Filter by target agent (free-form slug; exact match, case-insensitive).
    #[serde(alias = "to_machine")]
    pub to_agent: Option<String>,
    /// Filter by source agent (free-form slug; exact match, case-insensitive).
    #[serde(alias = "from_machine")]
    pub from_agent: Option<String>,
    /// Filter by category: "action" or "notify". Overrides include_notify
    /// when set. Omit to use the default action-only view.
    pub category: Option<String>,
    /// Include notify-class handoffs alongside action ones (default: false).
    /// Ignored when `category` is set explicitly.
    pub include_notify: Option<bool>,
    /// Max results (default 20, clamped to 1..=200)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
    /// Truncate body to 200 chars (default: true). Set false for full bodies.
    pub compact: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteHandoffParams {
    /// Handoff ID (UUID)
    pub handoff_id: String,
}

// ===== HANDOFF HANDLERS =====

/// Gate a by-id handoff operation for a caller whose token excludes client
/// data. A handoff the caller neither sent nor received reads as not found, so
/// the reply doesn't confirm that it exists. Unrestricted callers skip the
/// extra read. Sender and recipient never change after creation, so checking
/// before the write is not a race.
async fn ensure_handoff_visible(
    brain: &super::OpsBrain,
    id: uuid::Uuid,
    raw_id: &str,
    vis: crate::auth::Visibility<'_>,
) -> Result<(), CallToolResult> {
    if vis.party().is_none() {
        return Ok(());
    }
    match crate::repo::handoff_repo::get_handoff(&brain.pool, id).await {
        Ok(Some(h)) if vis.can_see_handoff(&h.from_agent, h.to_agent.as_deref()) => Ok(()),
        Ok(_) => Err(not_found("Handoff", raw_id)),
        Err(e) => Err(error_result(&format!("Database error: {e}"))),
    }
}

pub async fn handle_create_handoff(
    brain: &super::OpsBrain,
    p: CreateHandoffParams,
    bound: Option<&str>,
    vis: crate::auth::Visibility<'_>,
) -> CallToolResult {
    let priority = p.priority.as_deref().unwrap_or("normal");
    let category = p.category.as_deref().unwrap_or("action");

    if let Err(msg) = crate::validation::validate_bounded_text(
        &p.title,
        "title",
        crate::validation::MAX_TITLE_BYTES,
    ) {
        return error_result(&msg);
    }
    if let Err(msg) =
        crate::validation::validate_bounded_text(&p.body, "body", crate::validation::MAX_BODY_BYTES)
    {
        return error_result(&msg);
    }
    if let Some(context) = p.context.as_ref() {
        if let Err(msg) = crate::validation::validate_context(context) {
            return error_result(&msg);
        }
    }

    if let Err(msg) = crate::validation::validate_required(
        priority,
        "priority",
        crate::validation::HANDOFF_PRIORITIES,
    ) {
        return error_result(&msg);
    }
    if let Err(msg) = crate::validation::validate_required(
        category,
        "category",
        crate::validation::HANDOFF_CATEGORIES,
    ) {
        return error_result(&msg);
    }

    // Resolve optional reply-parent ID. Existence is enforced at the
    // database level by the FK; an invalid UUID returns 422-ish error
    // here, and a well-formed-but-missing UUID returns the FK violation
    // surfaced from the INSERT.
    let in_reply_to = match &p.in_reply_to {
        Some(id_str) => match uuid::Uuid::parse_str(id_str) {
            Ok(id) => Some(id),
            Err(_) => return error_result(&format!("Invalid in_reply_to UUID: {id_str}")),
        },
        None => None,
    };

    // Validate sender + target as free-form agent slugs. v2.0 dropped the
    // CC-fleet allowlist; whatever the caller says it is, it is. Stored
    // values are exact-match for `check_in` lookups, so writers and
    // readers must converge on the same string per agent.
    let from_agent = match crate::validation::validate_agent_name(&p.from_agent) {
        Ok(n) => n.to_string(),
        Err(e) => return error_result(&format!("from_agent: {e}")),
    };

    // Identity binding: a per-agent token may only file as its own slug. The
    // main bearer and stdio/dev are unbound and pass through unchanged.
    if let Err(msg) = crate::auth::check_bound_identity(bound, &from_agent) {
        return error_result(&msg);
    }

    let to_agent = match p.to_agent.as_deref() {
        Some(raw) => match crate::validation::validate_agent_name(raw) {
            Ok(n) => Some(n.to_string()),
            Err(e) => return error_result(&format!("to_agent: {e}")),
        },
        None => None,
    };
    // An action nobody is named on is work nobody owns. Only an announcement
    // can go to everyone.
    if to_agent.is_none() && category == "action" {
        return error_result(
            "to_agent is required for an action handoff; only a notify can be broadcast",
        );
    }

    let complete_parent = p.complete_parent.unwrap_or(false);
    if complete_parent && in_reply_to.is_none() {
        return error_result("complete_parent needs in_reply_to: there is no parent to complete");
    }
    let commit_hash = p
        .commit_hash
        .as_deref()
        .map(str::trim)
        .filter(|c| !c.is_empty());
    if commit_hash.is_some() && !complete_parent {
        return error_result(
            "commit_hash is recorded on the parent: set complete_parent, or pass it to \
             complete_handoff",
        );
    }

    if let (Some(parent), Some(raw)) = (in_reply_to, p.in_reply_to.as_deref()) {
        if let Err(r) = ensure_handoff_visible(brain, parent, raw, vis).await {
            return r;
        }
    }

    let new = crate::repo::handoff_repo::NewHandoff {
        from_agent: &from_agent,
        to_agent: to_agent.as_deref(),
        priority,
        category,
        title: &p.title,
        body: &p.body,
        context: p.context.as_ref(),
        in_reply_to,
    };
    let created = match in_reply_to.filter(|_| complete_parent) {
        Some(parent) => crate::repo::handoff_repo::create_reply_completing_parent(
            &brain.pool,
            &new,
            parent,
            commit_hash,
        )
        .await
        .map(|(reply, closed)| (reply, Some(closed.is_some()))),
        None => crate::repo::handoff_repo::create_handoff_from(&brain.pool, &new)
            .await
            .map(|reply| (reply, None)),
    };
    let (handoff, parent_closed) = match created {
        Ok(v) => v,
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    let text = crate::embeddings::prepare_handoff_text(&handoff);
    embed_and_store(
        &brain.pool,
        &brain.embedding_client,
        "handoffs",
        handoff.id,
        &text,
    )
    .await;

    let mut warnings: Vec<String> = Vec::new();
    if parent_closed == Some(false) {
        warnings.push(
            "reply filed, but the parent was not completed and no commit_hash was recorded: it \
             is already closed, or it is not addressed to or accepted by you (use \
             complete_handoff)"
                .to_string(),
        );
    }
    // The roster would describe the deployment to a caller that is kept
    // outside it, so a restricted caller gets no recipient check.
    if let (Some(target), None) = (to_agent.as_deref(), vis.party()) {
        warnings.extend(unbound_recipient_warning(&brain.known_agents, target));
    }

    if parent_closed.is_none() && warnings.is_empty() {
        return json_result(&handoff);
    }
    let mut value = match serde_json::to_value(&handoff) {
        Ok(v) => v,
        Err(_) => return json_result(&handoff),
    };
    if let Some(obj) = value.as_object_mut() {
        if let Some(closed) = parent_closed {
            obj.insert("_parent_completed".to_string(), closed.into());
        }
        if !warnings.is_empty() {
            obj.insert("_warning".to_string(), warnings.join("; ").into());
        }
    }
    json_result(&value)
}

/// Error for a family credential that claims without naming its member.
fn family_claim_hint(binding: &str) -> String {
    let base = binding.trim_end_matches("-*");
    format!(
        "family token {binding} must pass agent_name (your member name, e.g. {base}-<Bot>) \
         to accept_handoff"
    )
}

/// The most slugs a recipient warning lists before it stops being a hint.
const KNOWN_AGENTS_LISTED: usize = 20;

/// Soft warning for a recipient no credential on this deployment is bound to.
/// There is no agent registry by design, but the operator's static token
/// bindings already say who can read the bus: agent tokens, OAuth bindings,
/// and the queues machine tokens poll (which is how an operator slug or a
/// wake shim's agent gets here). A slug outside that set is a typo, a retired
/// seat, or a machine producer's sender name, and in each case no agent or
/// poller picks the handoff up (only a human reading the briefing would see
/// it). Never refuses the write. With no bindings configured
/// (stdio, dev, a main-bearer-only server) there is nothing to check against.
fn unbound_recipient_warning(known: &[String], target: &str) -> Option<String> {
    if known.is_empty() || known.iter().any(|b| crate::auth::slug_matches(b, target)) {
        return None;
    }
    let tail = if known.len() <= KNOWN_AGENTS_LISTED {
        format!("bound slugs: {}", known.join(", "))
    } else {
        format!("{} slugs are bound; check the spelling", known.len())
    };
    Some(format!(
        "no credential on this deployment is bound to '{target}', so no agent or poller will \
         pick this handoff up; {tail}"
    ))
}

pub async fn handle_get_handoff(
    brain: &super::OpsBrain,
    p: GetHandoffParams,
    vis: crate::auth::Visibility<'_>,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };

    match crate::repo::handoff_repo::get_handoff(&brain.pool, id).await {
        Ok(Some(handoff))
            if vis.can_see_handoff(&handoff.from_agent, handoff.to_agent.as_deref()) =>
        {
            json_result(&handoff)
        }
        Ok(_) => not_found("Handoff", &p.handoff_id),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_accept_handoff(
    brain: &super::OpsBrain,
    p: UpdateHandoffStatusParams,
    bound: Option<&str>,
    vis: crate::auth::Visibility<'_>,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };
    if let Err(r) = ensure_handoff_visible(brain, id, &p.handoff_id, vis).await {
        return r;
    }

    let supplied = match p.agent_name.as_deref() {
        Some(raw) => match crate::validation::validate_agent_name(raw) {
            Ok(name) => Some(name),
            Err(e) => return error_result(&format!("agent_name: {e}")),
        },
        None => None,
    };
    if let Some(name) = supplied {
        if let Err(msg) = crate::auth::check_bound_identity(bound, name) {
            return error_result(&msg);
        }
    }
    let acceptor = match bound {
        Some(binding) if binding.ends_with("-*") => match supplied {
            Some(name) => Some(name),
            None => return error_result(&family_claim_hint(binding)),
        },
        _ => bound.or(supplied),
    };
    // Atomic accept records the winner alongside the pending precondition.
    match crate::repo::handoff_repo::accept_handoff_as(&brain.pool, id, acceptor).await {
        Ok(Some(handoff)) => json_result(&handoff),
        Ok(None) => {
            // Lost the race or bad id — re-read to say which.
            match crate::repo::handoff_repo::get_handoff(&brain.pool, id).await {
                Ok(Some(h)) => {
                    error_result(&format!("Handoff is already '{}', cannot accept", h.status))
                }
                Ok(None) => not_found("Handoff", &p.handoff_id),
                Err(e) => error_result(&format!("Database error: {e}")),
            }
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_complete_handoff(
    brain: &super::OpsBrain,
    p: CompleteHandoffParams,
    vis: crate::auth::Visibility<'_>,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };
    if let Err(r) = ensure_handoff_visible(brain, id, &p.handoff_id, vis).await {
        return r;
    }

    // Atomic complete: only open (pending/accepted) rows transition, so a
    // concurrent complete can't silently overwrite commit_hash.
    match crate::repo::handoff_repo::complete_handoff_with_commit(
        &brain.pool,
        id,
        p.commit_hash.as_deref(),
    )
    .await
    {
        Ok(Some(handoff)) => json_result(&handoff),
        Ok(None) => match crate::repo::handoff_repo::get_handoff(&brain.pool, id).await {
            Ok(Some(h)) if h.status == "completed" => error_result("Handoff is already completed"),
            Ok(Some(h)) if h.status == "merged" => error_result("Handoff is already merged"),
            Ok(Some(h)) => error_result(&format!(
                "Handoff is '{}', cannot complete from that state",
                h.status
            )),
            Ok(None) => not_found("Handoff", &p.handoff_id),
            Err(e) => error_result(&format!("Database error: {e}")),
        },
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_list_handoffs(
    brain: &super::OpsBrain,
    p: ListHandoffsParams,
    vis: crate::auth::Visibility<'_>,
) -> CallToolResult {
    let page = crate::pagination::PageRequest::new(p.limit, 20);
    let compact = p.compact.unwrap_or(true);
    let include_notify = p.include_notify.unwrap_or(false);

    if let Err(msg) = crate::validation::validate_option(
        p.status.as_deref(),
        "status",
        crate::validation::HANDOFF_STATUSES,
    ) {
        return error_result(&msg);
    }
    if let Err(msg) = crate::validation::validate_option(
        p.category.as_deref(),
        "category",
        crate::validation::HANDOFF_CATEGORIES,
    ) {
        return error_result(&msg);
    }

    // Validate agent-name filters as free-form slugs (no normalization).
    // Rows are stored exactly as written; callers must match the stored form.
    let to_agent_filter = match p.to_agent.as_deref() {
        Some(raw) => match crate::validation::validate_agent_name(raw) {
            Ok(n) => Some(n.to_string()),
            Err(e) => return error_result(&format!("to_agent: {e}")),
        },
        None => None,
    };
    let from_agent_filter = match p.from_agent.as_deref() {
        Some(raw) => match crate::validation::validate_agent_name(raw) {
            Ok(n) => Some(n.to_string()),
            Err(e) => return error_result(&format!("from_agent: {e}")),
        },
        None => None,
    };

    match crate::repo::handoff_repo::list_handoffs(
        &brain.pool,
        p.status.as_deref(),
        to_agent_filter.as_deref(),
        from_agent_filter.as_deref(),
        p.category.as_deref(),
        include_notify,
        vis.party(),
        page.fetch_limit(),
    )
    .await
    {
        Ok(mut handoffs) => {
            let has_more = page.trim(&mut handoffs);
            if compact {
                let compacted: Vec<serde_json::Value> = handoffs
                    .iter()
                    .filter_map(|h| {
                        let mut val = serde_json::to_value(h).ok()?;
                        if let Some(obj) = val.as_object_mut() {
                            if let Some(serde_json::Value::String(body)) = obj.get("body") {
                                let truncated = if body.len() > 200 {
                                    format!("{}...", truncate_str(body, 200))
                                } else {
                                    body.clone()
                                };
                                obj.insert(
                                    "body".to_string(),
                                    serde_json::Value::String(truncated),
                                );
                            }
                        }
                        Some(val)
                    })
                    .collect();
                json_result(&serde_json::json!({
                    "handoffs": compacted,
                    "limit": page.limit,
                    "limit_clamped": page.limit_clamped,
                    "has_more": has_more,
                }))
            } else {
                json_result(&serde_json::json!({
                    "handoffs": handoffs,
                    "limit": page.limit,
                    "limit_clamped": page.limit_clamped,
                    "has_more": has_more,
                }))
            }
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_delete_handoff(
    brain: &super::OpsBrain,
    p: DeleteHandoffParams,
    bound: Option<&str>,
    vis: crate::auth::Visibility<'_>,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.handoff_id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.handoff_id)),
    };
    if let Err(r) = ensure_handoff_visible(brain, id, &p.handoff_id, vis).await {
        return r;
    }

    match crate::repo::handoff_repo::delete_handoff(&brain.pool, id, bound).await {
        Ok(true) => json_result(&serde_json::json!({"deleted": true, "id": p.handoff_id})),
        Ok(false) => not_found("Handoff", &p.handoff_id),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn create_handoff_accepts_legacy_machine_aliases() {
        let params: CreateHandoffParams = serde_json::from_value(serde_json::json!({
            "from_machine": "Claude-Laptop",
            "to_machine": "Codex-Buildbox",
            "title": "handoff",
            "body": "body"
        }))
        .unwrap();
        assert_eq!(params.from_agent, "Claude-Laptop");
        assert_eq!(params.to_agent.as_deref(), Some("Codex-Buildbox"));
    }

    #[test]
    fn list_handoffs_accepts_legacy_machine_aliases() {
        let params: ListHandoffsParams = serde_json::from_value(serde_json::json!({
            "from_machine": "Claude-Laptop",
            "to_machine": "Codex-Buildbox"
        }))
        .unwrap();
        assert_eq!(params.from_agent.as_deref(), Some("Claude-Laptop"));
        assert_eq!(params.to_agent.as_deref(), Some("Codex-Buildbox"));
    }
}
