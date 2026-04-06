//! REST API endpoints for ops-brain.
//!
//! These provide simple HTTP access to ops-brain data without requiring
//! MCP protocol negotiation. Protected by the same bearer auth middleware.

use axum::{extract::State, http::StatusCode, Json};
use serde::Deserialize;
use sqlx::PgPool;
use std::sync::Arc;

use crate::metrics::UptimeKumaConfig;
use crate::tools::briefings;
use crate::zammad::ZammadConfig;

/// Shared state for REST API handlers.
#[derive(Clone)]
pub struct ApiState {
    pub pool: PgPool,
    pub kuma_configs: Vec<UptimeKumaConfig>,
    pub zammad_config: Option<ZammadConfig>,
}

#[derive(Debug, Deserialize)]
pub struct GenerateBriefingRequest {
    /// "daily" or "weekly"
    #[serde(rename = "type")]
    pub briefing_type: String,
    /// Optional client slug to scope the briefing
    pub client_slug: Option<String>,
}

/// POST /api/briefing — generate and return an operational briefing.
pub async fn generate_briefing(
    State(state): State<Arc<ApiState>>,
    Json(req): Json<GenerateBriefingRequest>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let briefing_type = req.briefing_type.to_lowercase();
    if !["daily", "weekly"].contains(&briefing_type.as_str()) {
        return Err((
            StatusCode::BAD_REQUEST,
            format!(
                "Invalid type: '{}'. Use 'daily' or 'weekly'.",
                req.briefing_type
            ),
        ));
    }

    let client = match &req.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&state.pool, slug).await {
            Ok(Some(c)) => Some(c),
            Ok(None) => return Err((StatusCode::NOT_FOUND, format!("Client not found: {slug}"))),
            Err(e) => return Err((StatusCode::INTERNAL_SERVER_ERROR, format!("DB error: {e}"))),
        },
        None => None,
    };

    match generate_briefing_inner(
        &state.pool,
        &state.kuma_configs,
        &state.zammad_config,
        &briefing_type,
        client.as_ref(),
    )
    .await
    {
        Ok(data) => Ok(Json(data)),
        Err(e) => Err((StatusCode::INTERNAL_SERVER_ERROR, e)),
    }
}

/// Core briefing generation logic shared between the MCP tool and the REST API.
pub async fn generate_briefing_inner(
    pool: &PgPool,
    kuma_configs: &[UptimeKumaConfig],
    zammad_config: &Option<ZammadConfig>,
    briefing_type: &str,
    client: Option<&crate::models::client::Client>,
) -> Result<serde_json::Value, String> {
    let is_weekly = briefing_type == "weekly";
    let client_id = client.map(|c| c.id);
    let client_name = client.map(|c| c.name.as_str()).unwrap_or("All Clients");

    // ── Monitoring ──
    let monitoring_data = if !kuma_configs.is_empty() {
        match crate::metrics::fetch_all_metrics(kuma_configs).await {
            Ok(summary) => {
                let down_names: Vec<String> = summary
                    .monitors
                    .iter()
                    .filter(|m| m.status == 0)
                    .map(|m| m.name.clone())
                    .collect();
                briefings::MonitoringSummaryData {
                    status: if summary.down == 0 {
                        "ALL_CLEAR"
                    } else {
                        "DEGRADED"
                    }
                    .to_string(),
                    total: summary.total,
                    up: summary.up,
                    down: summary.down,
                    pending: summary.pending,
                    maintenance: summary.maintenance,
                    down_monitors: down_names,
                }
            }
            Err(e) => {
                tracing::warn!("Briefing: failed to fetch monitoring: {e}");
                briefings::MonitoringSummaryData {
                    status: "UNAVAILABLE".to_string(),
                    total: 0,
                    up: 0,
                    down: 0,
                    pending: 0,
                    maintenance: 0,
                    down_monitors: vec![],
                }
            }
        }
    } else {
        briefings::MonitoringSummaryData {
            status: "NOT_CONFIGURED".to_string(),
            total: 0,
            up: 0,
            down: 0,
            pending: 0,
            maintenance: 0,
            down_monitors: vec![],
        }
    };

    // ── Open incidents ──
    let open_incidents =
        crate::repo::incident_repo::list_incidents(pool, client_id, Some("open"), None, 100)
            .await
            .unwrap_or_default();

    let mut by_severity = std::collections::HashMap::new();
    for inc in &open_incidents {
        *by_severity.entry(inc.severity.clone()).or_insert(0usize) += 1;
    }
    let oldest_open = open_incidents
        .last()
        .map(|i| i.reported_at.format("%Y-%m-%d %H:%M UTC").to_string());

    // ── Watchdog incidents ──
    let watchdog_prefix = format!("{}%", crate::watchdog::INCIDENT_PREFIX);
    let watchdog_open: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM incidents WHERE title LIKE $1 AND status = 'open'",
    )
    .bind(&watchdog_prefix)
    .fetch_one(pool)
    .await
    .unwrap_or(0);

    let incident_data = briefings::IncidentSummaryData {
        open_total: open_incidents.len(),
        by_severity,
        oldest_open,
        watchdog_open: watchdog_open as usize,
    };

    // ── Pending handoffs ──
    // Briefings show actionable work only — notify-class FYIs are not "pending"
    // in any meaningful sense.
    let pending_handoffs = crate::repo::handoff_repo::list_handoffs(
        pool,
        Some("pending"),
        None,
        None,
        None,
        false,
        20,
    )
    .await
    .unwrap_or_default();

    let handoff_data = briefings::HandoffSummaryData {
        pending_count: pending_handoffs.len(),
        pending_titles: pending_handoffs.iter().map(|h| h.title.clone()).collect(),
    };

    // ── Zammad tickets ──
    let ticket_data = if let Some(zammad) = zammad_config {
        if let Some(c) = client {
            if let Some(org_id) = c.zammad_org_id {
                let open_query =
                    format!("organization.id:{org_id} AND (state.name:new OR state.name:open)");
                let open_tickets = crate::zammad::search_tickets(zammad, &open_query, 100)
                    .await
                    .unwrap_or_default();
                Some(ticket_summary(&open_tickets))
            } else {
                None
            }
        } else {
            let open_tickets =
                crate::zammad::search_tickets(zammad, "state.name:new OR state.name:open", 100)
                    .await
                    .unwrap_or_default();
            Some(ticket_summary(&open_tickets))
        }
    } else {
        None
    };

    // ── Weekly stats ──
    let weekly_stats = if is_weekly {
        let resolved: Vec<crate::models::incident::Incident> = sqlx::query_as(
            "SELECT * FROM incidents WHERE status = 'resolved' AND resolved_at >= NOW() - INTERVAL '7 days' ORDER BY resolved_at DESC",
        )
        .fetch_all(pool)
        .await
        .unwrap_or_default();

        let ttr_values: Vec<f64> = resolved
            .iter()
            .filter_map(|i| i.time_to_resolve_minutes.map(|t| t as f64))
            .collect();
        let avg_ttr = if ttr_values.is_empty() {
            None
        } else {
            Some(ttr_values.iter().sum::<f64>() / ttr_values.len() as f64)
        };

        let watchdog_resolved: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM incidents WHERE title LIKE $1 AND status = 'resolved' AND resolved_at >= NOW() - INTERVAL '7 days'",
        )
        .bind(&watchdog_prefix)
        .fetch_one(pool)
        .await
        .unwrap_or(0);

        Some(briefings::WeeklyStats {
            resolved_count: resolved.len(),
            avg_ttr_minutes: avg_ttr,
            watchdog_resolved: watchdog_resolved as usize,
        })
    } else {
        None
    };

    // ── Build markdown ──
    let now = chrono::Utc::now();
    let md = build_markdown(
        is_weekly,
        client_name,
        &now,
        &monitoring_data,
        &incident_data,
        &handoff_data,
        ticket_data.as_ref(),
        weekly_stats.as_ref(),
    );

    // Store
    let briefing = crate::repo::briefing_repo::insert_briefing(pool, briefing_type, client_id, &md)
        .await
        .map_err(|e| format!("Failed to store briefing: {e}"))?;

    let result = briefings::BriefingData {
        briefing_type: briefing_type.to_string(),
        client: client.map(|c| c.slug.clone()),
        generated_at: now.format("%Y-%m-%d %H:%M UTC").to_string(),
        monitoring: monitoring_data,
        incidents: incident_data,
        handoffs: handoff_data,
        tickets: ticket_data,
        weekly_stats,
        content: md,
    };

    let mut output = serde_json::to_value(&result).unwrap_or_default();
    output["briefing_id"] = serde_json::Value::String(briefing.id.to_string());
    Ok(output)
}

fn ticket_summary(tickets: &[crate::zammad::ZammadTicket]) -> briefings::TicketSummaryData {
    let new_count = tickets
        .iter()
        .filter(|t| t.state.as_deref() == Some("new"))
        .count();
    let mut by_priority = std::collections::HashMap::new();
    for t in tickets {
        let pri = t.priority.as_deref().unwrap_or("unknown").to_string();
        *by_priority.entry(pri).or_insert(0usize) += 1;
    }
    briefings::TicketSummaryData {
        open_count: tickets.len(),
        new_count,
        by_priority,
    }
}

#[allow(clippy::too_many_arguments)]
fn build_markdown(
    is_weekly: bool,
    client_name: &str,
    now: &chrono::DateTime<chrono::Utc>,
    monitoring: &briefings::MonitoringSummaryData,
    incidents: &briefings::IncidentSummaryData,
    handoffs: &briefings::HandoffSummaryData,
    tickets: Option<&briefings::TicketSummaryData>,
    weekly_stats: Option<&briefings::WeeklyStats>,
) -> String {
    let mut md = String::new();

    md.push_str(&format!(
        "# {} Operational Briefing — {}\n",
        if is_weekly { "Weekly" } else { "Daily" },
        client_name
    ));
    md.push_str(&format!(
        "*Generated: {}*\n\n",
        now.format("%Y-%m-%d %H:%M UTC")
    ));

    // Monitoring
    md.push_str("## Monitoring\n\n");
    match monitoring.status.as_str() {
        "ALL_CLEAR" => md.push_str(&format!(
            "**ALL CLEAR** — {}/{} monitors up\n\n",
            monitoring.up, monitoring.total
        )),
        "DEGRADED" => {
            md.push_str(&format!(
                "**DEGRADED** — {} DOWN out of {} monitors\n\n",
                monitoring.down, monitoring.total
            ));
            for name in &monitoring.down_monitors {
                md.push_str(&format!("- DOWN: {name}\n"));
            }
            md.push('\n');
        }
        _ => md.push_str(&format!("Status: {}\n\n", monitoring.status)),
    }
    if monitoring.maintenance > 0 {
        md.push_str(&format!("{} in maintenance\n\n", monitoring.maintenance));
    }

    // Incidents
    md.push_str("## Incidents\n\n");
    if incidents.open_total == 0 {
        md.push_str("No open incidents.\n\n");
    } else {
        md.push_str(&format!(
            "**{} open incident(s)**\n\n",
            incidents.open_total
        ));
        for sev in &["critical", "high", "medium", "low"] {
            if let Some(&count) = incidents.by_severity.get(*sev) {
                md.push_str(&format!("- {sev}: {count}\n"));
            }
        }
        if let Some(ref oldest) = incidents.oldest_open {
            md.push_str(&format!("\nOldest open since: {oldest}\n"));
        }
        md.push('\n');
        if incidents.watchdog_open > 0 {
            md.push_str(&format!(
                "{} auto-detected (watchdog) incident(s) still open\n\n",
                incidents.watchdog_open
            ));
        }
    }

    // Handoffs
    md.push_str("## Handoffs\n\n");
    if handoffs.pending_count == 0 {
        md.push_str("No pending handoffs.\n\n");
    } else {
        md.push_str(&format!(
            "**{} pending handoff(s)**\n\n",
            handoffs.pending_count
        ));
        for title in &handoffs.pending_titles {
            md.push_str(&format!("- {title}\n"));
        }
        md.push('\n');
    }

    // Tickets
    if let Some(tickets) = tickets {
        md.push_str("## Tickets\n\n");
        if tickets.open_count == 0 {
            md.push_str("No open tickets.\n\n");
        } else {
            md.push_str(&format!(
                "**{} open ticket(s)** ({} new)\n\n",
                tickets.open_count, tickets.new_count
            ));
            for (pri, count) in &tickets.by_priority {
                md.push_str(&format!("- {pri}: {count}\n"));
            }
            md.push('\n');
        }
    }

    // Weekly stats
    if let Some(stats) = weekly_stats {
        md.push_str("## Weekly Stats (last 7 days)\n\n");
        md.push_str(&format!("- Incidents resolved: {}\n", stats.resolved_count));
        if let Some(avg) = stats.avg_ttr_minutes {
            md.push_str(&format!("- Avg time to resolve: {:.0} min\n", avg));
        }
        if stats.watchdog_resolved > 0 {
            md.push_str(&format!(
                "- Watchdog auto-resolved: {}\n",
                stats.watchdog_resolved
            ));
        }
        md.push('\n');
    }

    md
}
