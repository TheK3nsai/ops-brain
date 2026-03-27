//! Proactive monitoring watchdog (Phase 6).
//!
//! Polls Uptime Kuma on a configurable interval, detects monitor state transitions
//! (UP→DOWN / DOWN→UP), and automatically creates/resolves incidents with linked
//! servers and services.
//!
//! ## Noise Reduction
//!
//! Three mechanisms prevent noisy monitors from flooding the incident list:
//!
//! 1. **Grace period** (`confirm_polls`): A monitor must report DOWN for N consecutive
//!    polls before an incident is created. With the default interval of 60s and
//!    confirm_polls=3, a monitor must be down for ~3 minutes. This handles push-monitor
//!    heartbeat timing jitter and brief transient blips.
//!
//! 2. **Cooldown** (`cooldown_secs`): After auto-resolving an incident, the watchdog
//!    will not create a new incident for the same monitor for N seconds (default 1800 =
//!    30 minutes). This handles monitors that flap DOWN→UP→DOWN repeatedly.
//!
//! 3. **Deduplication**: Before creating a new incident, the watchdog checks for a
//!    recently resolved incident (within 24h) with the same title. If found, it reopens
//!    the existing incident and increments `recurrence_count` instead of creating a new
//!    one. This prevents recurring heartbeat misses (e.g. push monitors) from creating
//!    10+ identical incidents per day.

use std::collections::HashMap;

use sqlx::PgPool;
use tokio::time::Instant;
use uuid::Uuid;

use crate::embeddings::EmbeddingClient;
use crate::metrics::{self, MonitorStatus, UptimeKumaConfig};

/// Prefix used in incident titles so the watchdog can find its own incidents.
pub const INCIDENT_PREFIX: &str = "[AUTO] ";

/// How the watchdog identifies incidents it created for a specific monitor.
fn watchdog_incident_title(monitor_name: &str, status_text: &str) -> String {
    format!("{INCIDENT_PREFIX}Monitor {status_text}: {monitor_name}")
}

/// Bundled watchdog configuration (avoids parameter bloat on `run()`).
#[derive(Debug, Clone)]
pub struct WatchdogConfig {
    pub interval_secs: u64,
    /// Consecutive DOWN polls required before creating an incident.
    pub confirm_polls: u32,
    /// Seconds to suppress new incidents for a monitor after its last incident was resolved.
    pub cooldown_secs: u64,
}

/// Tracks the last-known status of each monitor.
#[derive(Debug, Clone)]
struct MonitorState {
    status: i64,
    /// If this monitor is currently DOWN and the watchdog created an incident, store its ID.
    incident_id: Option<Uuid>,
    /// How many consecutive polls this monitor has reported DOWN.
    consecutive_down: u32,
    /// When the last auto-created incident for this monitor was resolved (for cooldown).
    resolved_at: Option<Instant>,
}

impl MonitorState {
    fn new(status: i64) -> Self {
        Self {
            status,
            incident_id: None,
            consecutive_down: 0,
            resolved_at: None,
        }
    }

    fn new_with_incident(incident_id: Uuid) -> Self {
        Self {
            status: 0,
            incident_id: Some(incident_id),
            // Already confirmed — recovered from DB
            consecutive_down: u32::MAX,
            resolved_at: None,
        }
    }

    /// Returns true if this monitor is in a post-resolution cooldown period.
    fn in_cooldown(&self, cooldown_secs: u64) -> bool {
        if cooldown_secs == 0 {
            return false;
        }
        match self.resolved_at {
            Some(resolved) => resolved.elapsed().as_secs() < cooldown_secs,
            None => false,
        }
    }
}

/// The watchdog background task.
pub async fn run(
    pool: PgPool,
    kuma_config: UptimeKumaConfig,
    embedding_client: Option<EmbeddingClient>,
    watchdog_config: WatchdogConfig,
) {
    tracing::info!(
        interval_secs = watchdog_config.interval_secs,
        confirm_polls = watchdog_config.confirm_polls,
        cooldown_secs = watchdog_config.cooldown_secs,
        "Watchdog started — polling Uptime Kuma every {}s (confirm after {} polls, {}s cooldown)",
        watchdog_config.interval_secs,
        watchdog_config.confirm_polls,
        watchdog_config.cooldown_secs,
    );

    let mut states: HashMap<String, MonitorState> = HashMap::new();

    // On first poll, load existing open watchdog incidents to recover state
    recover_state(&pool, &mut states).await;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(
            watchdog_config.interval_secs,
        ))
        .await;

        match metrics::fetch_metrics(&kuma_config).await {
            Ok(summary) => {
                tracing::debug!(
                    total = summary.total,
                    up = summary.up,
                    down = summary.down,
                    "Watchdog poll complete"
                );
                process_monitors(
                    &pool,
                    &embedding_client,
                    &mut states,
                    summary.monitors,
                    &watchdog_config,
                )
                .await;
            }
            Err(e) => {
                tracing::error!("Watchdog failed to fetch metrics: {e}");
            }
        }
    }
}

/// On startup, find open incidents created by the watchdog and restore the state map.
async fn recover_state(pool: &PgPool, states: &mut HashMap<String, MonitorState>) {
    let prefix_pattern = format!("{INCIDENT_PREFIX}Monitor DOWN: %");
    let rows = sqlx::query_as::<_, crate::models::incident::Incident>(
        "SELECT * FROM incidents WHERE title LIKE $1 AND status = 'open'",
    )
    .bind(&prefix_pattern)
    .fetch_all(pool)
    .await;

    match rows {
        Ok(incidents) => {
            for incident in incidents {
                // Extract monitor name from title: "[AUTO] Monitor DOWN: Some Monitor"
                if let Some(monitor_name) = incident
                    .title
                    .strip_prefix(&format!("{INCIDENT_PREFIX}Monitor DOWN: "))
                {
                    tracing::info!(
                        monitor = monitor_name,
                        incident_id = %incident.id,
                        "Watchdog recovered open incident"
                    );
                    states.insert(
                        monitor_name.to_string(),
                        MonitorState::new_with_incident(incident.id),
                    );
                }
            }
            if !states.is_empty() {
                tracing::info!(
                    count = states.len(),
                    "Watchdog recovered {} open incident(s) from previous run",
                    states.len()
                );
            }
        }
        Err(e) => {
            tracing::warn!("Watchdog failed to recover state: {e}");
        }
    }
}

/// Compare current monitor statuses against last-known state and act on transitions.
async fn process_monitors(
    pool: &PgPool,
    embedding_client: &Option<EmbeddingClient>,
    states: &mut HashMap<String, MonitorState>,
    monitors: Vec<MonitorStatus>,
    config: &WatchdogConfig,
) {
    for monitor in monitors {
        let name = &monitor.name;
        let new_status = monitor.status;

        let prev = states.get(name);
        let prev_status = prev.map(|s| s.status);
        let prev_incident_id = prev.and_then(|s| s.incident_id);
        let prev_consecutive_down = prev.map(|s| s.consecutive_down).unwrap_or(0);
        let prev_resolved_at = prev.and_then(|s| s.resolved_at);

        match (prev_status, new_status) {
            // ── First poll — seed state ─────────────────────────────────────
            (None, 0) => {
                // Already DOWN on first poll — start counting toward threshold
                let mut state = MonitorState::new(0);
                state.consecutive_down = 1;

                if config.confirm_polls <= 1 {
                    // Threshold is 1 (or 0) — create immediately
                    let incident_id =
                        handle_down_transition(pool, embedding_client, name, &monitor).await;
                    state.incident_id = incident_id;
                } else {
                    tracing::info!(
                        monitor = %name,
                        consecutive_down = 1,
                        threshold = config.confirm_polls,
                        "Monitor is DOWN on first poll — waiting for confirmation ({}/{})",
                        1, config.confirm_polls,
                    );
                }
                states.insert(name.clone(), state);
            }
            (None, _) => {
                states.insert(name.clone(), MonitorState::new(new_status));
            }

            // ── Transition to DOWN (was UP/PENDING/MAINTENANCE) ─────────────
            (Some(prev), 0) if prev != 0 => {
                let mut state = MonitorState::new(0);
                state.consecutive_down = 1;
                state.resolved_at = prev_resolved_at;

                if config.confirm_polls <= 1 && !state.in_cooldown(config.cooldown_secs) {
                    tracing::warn!(monitor = %name, prev_status = prev, "Monitor went DOWN");
                    let incident_id =
                        handle_down_transition(pool, embedding_client, name, &monitor).await;
                    state.incident_id = incident_id;
                } else if state.in_cooldown(config.cooldown_secs) {
                    tracing::info!(
                        monitor = %name,
                        "Monitor went DOWN but is in cooldown — suppressing (flap detected)"
                    );
                } else {
                    tracing::info!(
                        monitor = %name,
                        consecutive_down = 1,
                        threshold = config.confirm_polls,
                        "Monitor went DOWN — waiting for confirmation ({}/{})",
                        1, config.confirm_polls,
                    );
                }
                states.insert(name.clone(), state);
            }

            // ── Still DOWN — increment counter, maybe create incident ───────
            (Some(0), 0) => {
                let count = prev_consecutive_down.saturating_add(1);
                let mut state = MonitorState {
                    status: 0,
                    incident_id: prev_incident_id,
                    consecutive_down: count,
                    resolved_at: prev_resolved_at,
                };

                // Threshold reached AND no incident yet AND not in cooldown
                if count >= config.confirm_polls
                    && state.incident_id.is_none()
                    && !state.in_cooldown(config.cooldown_secs)
                {
                    tracing::warn!(
                        monitor = %name,
                        consecutive_down = count,
                        "Monitor confirmed DOWN after {} consecutive polls — creating incident",
                        count,
                    );
                    let incident_id =
                        handle_down_transition(pool, embedding_client, name, &monitor).await;
                    state.incident_id = incident_id;
                } else if count >= config.confirm_polls
                    && state.incident_id.is_none()
                    && state.in_cooldown(config.cooldown_secs)
                {
                    tracing::info!(
                        monitor = %name,
                        consecutive_down = count,
                        "Monitor confirmed DOWN but in cooldown — suppressing (flap detected)"
                    );
                } else if state.incident_id.is_none() && count < config.confirm_polls {
                    tracing::debug!(
                        monitor = %name,
                        consecutive_down = count,
                        threshold = config.confirm_polls,
                        "Monitor still DOWN — waiting ({}/{})",
                        count, config.confirm_polls,
                    );
                }
                // If incident already exists, just keep counting (no action)

                states.insert(name.clone(), state);
            }

            // ── DOWN → UP — resolve incident ────────────────────────────────
            (Some(0), 1) => {
                tracing::info!(monitor = %name, "Monitor recovered (UP)");
                let resolved_at = if let Some(incident_id) = prev_incident_id {
                    handle_up_transition(pool, incident_id, name).await;
                    Some(Instant::now())
                } else {
                    // Was counting toward threshold but never created an incident
                    if prev_consecutive_down > 0 {
                        tracing::info!(
                            monitor = %name,
                            consecutive_down = prev_consecutive_down,
                            "Monitor recovered before incident threshold — no incident to resolve"
                        );
                    }
                    prev_resolved_at
                };
                states.insert(
                    name.clone(),
                    MonitorState {
                        status: 1,
                        incident_id: None,
                        consecutive_down: 0,
                        resolved_at,
                    },
                );
            }

            // ── No change or irrelevant transition ──────────────────────────
            _ => {
                let consecutive_down = if new_status == 0 {
                    prev_consecutive_down
                } else {
                    0
                };
                states.insert(
                    name.clone(),
                    MonitorState {
                        status: new_status,
                        incident_id: prev_incident_id,
                        consecutive_down,
                        resolved_at: prev_resolved_at,
                    },
                );
            }
        }
    }
}

/// Handle a monitor going DOWN: create incident, link server/service, find runbooks.
async fn handle_down_transition(
    pool: &PgPool,
    embedding_client: &Option<EmbeddingClient>,
    monitor_name: &str,
    monitor_status: &MonitorStatus,
) -> Option<Uuid> {
    // Look up the monitor mapping in the DB
    let monitor_mapping = crate::repo::monitor_repo::get_monitor_by_name(pool, monitor_name)
        .await
        .ok()
        .flatten();

    // Resolve server and client for the incident
    let (server_id, service_id, client_id, severity) = if let Some(ref mapping) = monitor_mapping {
        let server = if let Some(sid) = mapping.server_id {
            crate::repo::server_repo::get_server(pool, sid)
                .await
                .ok()
                .flatten()
        } else {
            None
        };

        let client_id = if let Some(ref srv) = server {
            // Walk server → site → client
            let site = crate::repo::site_repo::get_site(pool, srv.site_id)
                .await
                .ok()
                .flatten();
            site.map(|s| s.client_id)
        } else {
            None
        };

        // Determine severity based on monitor type and server roles
        let severity = determine_severity(server.as_ref());

        (mapping.server_id, mapping.service_id, client_id, severity)
    } else {
        (None, None, None, "medium".to_string())
    };

    // Build symptoms description
    let symptoms = format!(
        "Uptime Kuma monitor '{}' reports DOWN.\nType: {}\nURL: {}\nHostname: {}\nPort: {}\nResponse time: {}",
        monitor_name,
        monitor_status.monitor_type,
        if monitor_status.url.is_empty() { "N/A" } else { &monitor_status.url },
        if monitor_status.hostname.is_empty() { "N/A" } else { &monitor_status.hostname },
        if monitor_status.port.is_empty() { "N/A" } else { &monitor_status.port },
        monitor_status
            .response_time_ms
            .map(|ms| format!("{ms}ms"))
            .unwrap_or_else(|| "N/A".to_string()),
    );

    let title = watchdog_incident_title(monitor_name, "DOWN");

    // Deduplication: check for a recently resolved incident for the same monitor.
    // If found, reopen it instead of creating a new one (reduces noise from recurring heartbeat misses).
    let incident = match crate::repo::incident_repo::find_recent_resolved_watchdog_incident(
        pool, &title,
    )
    .await
    {
        Ok(Some(existing)) => {
            let recurrence = existing.recurrence_count + 1;
            let reopen_note = format!(
                "Recurrence #{recurrence}: monitor went DOWN again at {}. Reopened by watchdog.",
                chrono::Utc::now().format("%Y-%m-%d %H:%M UTC")
            );
            match crate::repo::incident_repo::reopen_incident(
                pool,
                existing.id,
                Some(&symptoms),
                &reopen_note,
            )
            .await
            {
                Ok(reopened) => {
                    tracing::warn!(
                        monitor = %monitor_name,
                        incident_id = %reopened.id,
                        recurrence_count = reopened.recurrence_count,
                        "Reopened existing incident (recurrence #{}) instead of creating new",
                        reopened.recurrence_count,
                    );
                    reopened
                }
                Err(e) => {
                    tracing::error!(monitor = %monitor_name, "Failed to reopen incident: {e}");
                    return None;
                }
            }
        }
        _ => {
            // No recent incident found — create a new one
            let incident = crate::repo::incident_repo::create_incident_with_source(
                pool,
                &title,
                &severity,
                client_id,
                Some(&symptoms),
                Some("Auto-created by ops-brain watchdog on monitor DOWN transition"),
                false, // cross_client_safe: watchdog incidents are client-scoped by default
                Some("watchdog"),
            )
            .await;

            match incident {
                Ok(inc) => {
                    tracing::warn!(
                        monitor = %monitor_name,
                        incident_id = %inc.id,
                        severity = %severity,
                        "Created incident for DOWN monitor"
                    );
                    inc
                }
                Err(e) => {
                    tracing::error!(monitor = %monitor_name, "Failed to create watchdog incident: {e}");
                    return None;
                }
            }
        }
    };

    // Link server and service
    if let Some(sid) = server_id {
        let _ = crate::repo::incident_repo::link_incident_server(pool, incident.id, sid).await;
    }
    if let Some(sid) = service_id {
        let _ = crate::repo::incident_repo::link_incident_service(pool, incident.id, sid).await;
    }

    // Find and log relevant runbooks via semantic search (client-scoped)
    suggest_runbooks(
        pool,
        embedding_client,
        &incident.id,
        monitor_name,
        &symptoms,
        client_id,
    )
    .await;

    // Embed the incident (best-effort)
    if let Some(ref client) = embedding_client {
        let text = crate::embeddings::prepare_incident_text(&incident);
        if let Ok(embedding) = client.embed_text(&text).await {
            let _ = crate::repo::embedding_repo::store_incident_embedding(
                pool,
                incident.id,
                &embedding,
            )
            .await;
        }
    }

    Some(incident.id)
}

/// Handle a monitor recovering (DOWN→UP): auto-resolve the linked incident.
async fn handle_up_transition(pool: &PgPool, incident_id: Uuid, monitor_name: &str) {
    let result = crate::repo::incident_repo::update_incident(
        pool,
        incident_id,
        None, // title
        Some("resolved"),
        None, // severity
        None, // symptoms
        Some("Monitor recovered automatically"),
        Some(&format!(
            "Uptime Kuma monitor '{}' returned to UP status. Auto-resolved by watchdog.",
            monitor_name
        )),
        None, // prevention
        None, // notes
        None, // cross_client_safe (preserve existing value)
    )
    .await;

    match result {
        Ok(inc) => {
            tracing::info!(
                monitor = %monitor_name,
                incident_id = %incident_id,
                ttr_minutes = inc.time_to_resolve_minutes,
                "Auto-resolved incident — monitor recovered"
            );
        }
        Err(e) => {
            tracing::error!(
                monitor = %monitor_name,
                incident_id = %incident_id,
                "Failed to auto-resolve incident: {e}"
            );
        }
    }
}

/// Determine incident severity based on server roles and criticality.
fn determine_severity(server: Option<&crate::models::server::Server>) -> String {
    let Some(server) = server else {
        return "medium".to_string();
    };

    // Domain controllers are always critical
    for role in &server.roles {
        let role_lower = role.to_lowercase();
        if role_lower.contains("domain-controller")
            || role_lower.contains("dc")
            || role_lower.contains("dns")
            || role_lower.contains("dhcp")
        {
            return "critical".to_string();
        }
    }

    // Check for high-impact roles
    for role in &server.roles {
        let role_lower = role.to_lowercase();
        if role_lower.contains("file-server")
            || role_lower.contains("rds")
            || role_lower.contains("database")
            || role_lower.contains("backup")
        {
            return "high".to_string();
        }
    }

    "medium".to_string()
}

/// Use semantic search to find relevant runbooks and log them as suggestions.
/// Only suggests runbooks from the same client or global (no client) — prevents cross-client leakage.
async fn suggest_runbooks(
    pool: &PgPool,
    embedding_client: &Option<EmbeddingClient>,
    incident_id: &Uuid,
    monitor_name: &str,
    symptoms: &str,
    client_id: Option<Uuid>,
) {
    // Try semantic search first, fall back to FTS
    let query_text = format!("{monitor_name} {symptoms}");

    let embedding = if let Some(ref client) = embedding_client {
        client.embed_text(&query_text).await.ok()
    } else {
        None
    };

    let all_runbooks = crate::repo::embedding_repo::hybrid_search_runbooks(
        pool,
        &query_text,
        embedding.as_deref(),
        10, // fetch more, then filter by client scope
    )
    .await
    .unwrap_or_default();

    // Filter to same-client or global runbooks only
    let runbooks: Vec<_> = all_runbooks
        .into_iter()
        .filter(|r| match (client_id, r.client_id) {
            (_, None) => true,                            // Global runbook — always OK
            (Some(req), Some(own)) if req == own => true, // Same client
            (Some(_), Some(_)) => r.cross_client_safe,    // Different client — only if marked safe
            (None, Some(_)) => true,                      // No requesting client — allow all
        })
        .take(3)
        .collect();

    if !runbooks.is_empty() {
        let suggestions: Vec<String> = runbooks
            .iter()
            .map(|r| format!("  - {} ({})", r.title, r.slug))
            .collect();
        tracing::info!(
            monitor = %monitor_name,
            incident_id = %incident_id,
            "Suggested runbooks:\n{}",
            suggestions.join("\n")
        );

        // Auto-link runbooks to the incident (usage: not-applicable until followed)
        for runbook in &runbooks {
            let _ = crate::repo::incident_repo::link_incident_runbook(
                pool,
                *incident_id,
                runbook.id,
                "not-applicable",
            )
            .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_server(roles: Vec<&str>) -> crate::models::server::Server {
        crate::models::server::Server {
            id: Uuid::now_v7(),
            site_id: Uuid::now_v7(),
            hostname: "test-server".to_string(),
            slug: "test-server".to_string(),
            os: None,
            ip_addresses: vec![],
            ssh_alias: None,
            roles: roles.into_iter().map(String::from).collect(),
            hardware: None,
            cpu: None,
            ram_gb: None,
            storage_summary: None,
            is_virtual: false,
            hypervisor_id: None,
            status: "active".to_string(),
            notes: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
        }
    }

    #[test]
    fn severity_no_server_is_medium() {
        assert_eq!(determine_severity(None), "medium");
    }

    #[test]
    fn severity_domain_controller_is_critical() {
        let server = make_server(vec!["domain-controller"]);
        assert_eq!(determine_severity(Some(&server)), "critical");
    }

    #[test]
    fn severity_dc_is_critical() {
        let server = make_server(vec!["dc"]);
        assert_eq!(determine_severity(Some(&server)), "critical");
    }

    #[test]
    fn severity_dns_is_critical() {
        let server = make_server(vec!["dns"]);
        assert_eq!(determine_severity(Some(&server)), "critical");
    }

    #[test]
    fn severity_dhcp_is_critical() {
        let server = make_server(vec!["dhcp"]);
        assert_eq!(determine_severity(Some(&server)), "critical");
    }

    #[test]
    fn severity_file_server_is_high() {
        let server = make_server(vec!["file-server"]);
        assert_eq!(determine_severity(Some(&server)), "high");
    }

    #[test]
    fn severity_rds_is_high() {
        let server = make_server(vec!["rds"]);
        assert_eq!(determine_severity(Some(&server)), "high");
    }

    #[test]
    fn severity_database_is_high() {
        let server = make_server(vec!["database"]);
        assert_eq!(determine_severity(Some(&server)), "high");
    }

    #[test]
    fn severity_backup_is_high() {
        let server = make_server(vec!["backup"]);
        assert_eq!(determine_severity(Some(&server)), "high");
    }

    #[test]
    fn severity_print_server_is_medium() {
        let server = make_server(vec!["print-server"]);
        assert_eq!(determine_severity(Some(&server)), "medium");
    }

    #[test]
    fn severity_multiple_roles_critical_wins() {
        let server = make_server(vec!["file-server", "dns"]);
        assert_eq!(determine_severity(Some(&server)), "critical");
    }

    #[test]
    fn severity_case_insensitive() {
        let server = make_server(vec!["DNS"]);
        assert_eq!(determine_severity(Some(&server)), "critical");

        let server = make_server(vec!["File-Server"]);
        assert_eq!(determine_severity(Some(&server)), "high");
    }

    #[test]
    fn watchdog_incident_title_format() {
        assert_eq!(
            watchdog_incident_title("Nextcloud", "DOWN"),
            "[AUTO] Monitor DOWN: Nextcloud"
        );
        assert_eq!(
            watchdog_incident_title("SSH", "UP"),
            "[AUTO] Monitor UP: SSH"
        );
    }

    // ── Flap suppression unit tests ─────────────────────────────────────

    #[test]
    fn monitor_state_new_defaults() {
        let state = MonitorState::new(1);
        assert_eq!(state.status, 1);
        assert!(state.incident_id.is_none());
        assert_eq!(state.consecutive_down, 0);
        assert!(state.resolved_at.is_none());
    }

    #[test]
    fn monitor_state_new_with_incident() {
        let id = Uuid::now_v7();
        let state = MonitorState::new_with_incident(id);
        assert_eq!(state.status, 0);
        assert_eq!(state.incident_id, Some(id));
        assert_eq!(state.consecutive_down, u32::MAX);
        assert!(state.resolved_at.is_none());
    }

    #[test]
    fn cooldown_zero_always_false() {
        let mut state = MonitorState::new(1);
        state.resolved_at = Some(Instant::now());
        assert!(!state.in_cooldown(0));
    }

    #[test]
    fn cooldown_no_resolved_at_is_false() {
        let state = MonitorState::new(1);
        assert!(!state.in_cooldown(1800));
    }

    #[test]
    fn cooldown_active_when_recent() {
        let mut state = MonitorState::new(1);
        state.resolved_at = Some(Instant::now());
        // Just resolved — should be in cooldown
        assert!(state.in_cooldown(1800));
    }

    #[test]
    fn cooldown_expired() {
        let mut state = MonitorState::new(1);
        // Set resolved_at to 1 second ago
        state.resolved_at = Some(Instant::now() - std::time::Duration::from_secs(2));
        // Cooldown is 1 second — should be expired
        assert!(!state.in_cooldown(1));
    }
}
