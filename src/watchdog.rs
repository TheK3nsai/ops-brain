//! Proactive monitoring watchdog (Phase 6).
//!
//! Polls Uptime Kuma on a configurable interval, detects monitor state transitions
//! (UP→DOWN / DOWN→UP), and automatically creates/resolves incidents with linked
//! servers and services.

use std::collections::HashMap;

use sqlx::PgPool;
use uuid::Uuid;

use crate::embeddings::EmbeddingClient;
use crate::metrics::{self, MonitorStatus, UptimeKumaConfig};

/// Prefix used in incident titles so the watchdog can find its own incidents.
pub const INCIDENT_PREFIX: &str = "[AUTO] ";

/// How the watchdog identifies incidents it created for a specific monitor.
fn watchdog_incident_title(monitor_name: &str, status_text: &str) -> String {
    format!("{INCIDENT_PREFIX}Monitor {status_text}: {monitor_name}")
}

/// Tracks the last-known status of each monitor.
#[derive(Debug, Clone)]
struct MonitorState {
    status: i64,
    /// If this monitor is currently DOWN and the watchdog created an incident, store its ID.
    incident_id: Option<Uuid>,
}

/// The watchdog background task.
pub async fn run(
    pool: PgPool,
    kuma_config: UptimeKumaConfig,
    embedding_client: Option<EmbeddingClient>,
    interval_secs: u64,
) {
    tracing::info!(
        interval_secs,
        "Watchdog started — polling Uptime Kuma every {interval_secs}s"
    );

    let mut states: HashMap<String, MonitorState> = HashMap::new();

    // On first poll, load existing open watchdog incidents to recover state
    recover_state(&pool, &mut states).await;

    loop {
        tokio::time::sleep(std::time::Duration::from_secs(interval_secs)).await;

        match metrics::fetch_metrics(&kuma_config).await {
            Ok(summary) => {
                tracing::debug!(
                    total = summary.total,
                    up = summary.up,
                    down = summary.down,
                    "Watchdog poll complete"
                );
                process_monitors(&pool, &embedding_client, &mut states, summary.monitors).await;
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
                        MonitorState {
                            status: 0, // DOWN
                            incident_id: Some(incident.id),
                        },
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
) {
    for monitor in monitors {
        let name = &monitor.name;
        let new_status = monitor.status;

        let prev = states.get(name);
        let prev_status = prev.map(|s| s.status);
        let prev_incident_id = prev.and_then(|s| s.incident_id);

        match (prev_status, new_status) {
            // First poll — seed state, no transition
            (None, _) => {
                states.insert(
                    name.clone(),
                    MonitorState {
                        status: new_status,
                        incident_id: None,
                    },
                );
                if new_status == 0 {
                    tracing::warn!(
                        monitor = %name,
                        "Watchdog initial poll: monitor is already DOWN"
                    );
                    // Create incident for monitors that are already down on first poll
                    let incident_id =
                        handle_down_transition(pool, embedding_client, name, &monitor).await;
                    if let Some(id) = incident_id {
                        states.get_mut(name).unwrap().incident_id = Some(id);
                    }
                }
            }

            // Was UP/PENDING/MAINTENANCE, now DOWN → create incident
            (Some(prev), 0) if prev != 0 => {
                tracing::warn!(
                    monitor = %name,
                    prev_status = prev,
                    "Monitor went DOWN"
                );
                let incident_id =
                    handle_down_transition(pool, embedding_client, name, &monitor).await;
                states.insert(
                    name.clone(),
                    MonitorState {
                        status: 0,
                        incident_id,
                    },
                );
            }

            // Was DOWN, now UP → resolve incident
            (Some(0), 1) => {
                tracing::info!(
                    monitor = %name,
                    "Monitor recovered (UP)"
                );
                if let Some(incident_id) = prev_incident_id {
                    handle_up_transition(pool, incident_id, name).await;
                }
                states.insert(
                    name.clone(),
                    MonitorState {
                        status: 1,
                        incident_id: None,
                    },
                );
            }

            // No change or irrelevant transition
            _ => {
                states.insert(
                    name.clone(),
                    MonitorState {
                        status: new_status,
                        incident_id: prev_incident_id,
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

    // Create the incident
    let incident = crate::repo::incident_repo::create_incident(
        pool,
        &title,
        &severity,
        client_id,
        Some(&symptoms),
        Some("Auto-created by ops-brain watchdog on monitor DOWN transition"),
    )
    .await;

    let incident = match incident {
        Ok(inc) => inc,
        Err(e) => {
            tracing::error!(monitor = %monitor_name, "Failed to create watchdog incident: {e}");
            return None;
        }
    };

    tracing::warn!(
        monitor = %monitor_name,
        incident_id = %incident.id,
        severity = %severity,
        "Created incident for DOWN monitor"
    );

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
        // If any role is critical, the whole server is critical
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
}
