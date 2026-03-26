use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{error_result, json_result, not_found, not_found_with_suggestions};
use crate::models::incident::Incident;
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListMonitorsParams {
    /// Filter by status: "up", "down", "pending", "maintenance" (optional)
    pub status: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMonitorStatusParams {
    /// The exact monitor name as it appears in Uptime Kuma
    pub monitor_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GetMonitoringSummaryParams {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct LinkMonitorParams {
    /// The exact monitor name as it appears in Uptime Kuma
    pub monitor_name: String,
    /// Server slug to link this monitor to (optional)
    pub server_slug: Option<String>,
    /// Service slug to link this monitor to (optional)
    pub service_slug: Option<String>,
    /// Notes about what this monitor watches (optional)
    pub notes: Option<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct UnlinkMonitorParams {
    /// The exact monitor name to remove the mapping for
    pub monitor_name: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListWatchdogIncidentsParams {
    /// Filter by status: "open" or "resolved" (default: all)
    pub status: Option<String>,
    /// Maximum number of incidents to return (default: 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

/// Inject a diagnostic hint for push-type monitors that are DOWN.
/// Push monitors report "down" when their heartbeat expires (cron/script issue),
/// not necessarily when the underlying service is actually down.
fn inject_push_diagnostic_hint(
    val: &mut serde_json::Value,
    monitor: &crate::metrics::MonitorStatus,
) {
    if monitor.monitor_type == "push" && monitor.status == 0 {
        val["diagnostic_hint"] = serde_json::Value::String(
            "Push monitor — DOWN usually means heartbeat expired (cron/script issue), \
             not service failure. Check cron schedule and script logs on the target server."
                .to_string(),
        );
    }
}

// ===== HANDLERS =====

pub(crate) async fn handle_list_monitors(
    brain: &super::OpsBrain,
    p: ListMonitorsParams,
) -> CallToolResult {
    let kuma_config = match &brain.kuma_config {
        Some(c) => c,
        None => return error_result("Uptime Kuma not configured (set UPTIME_KUMA_URL)"),
    };

    let summary = match crate::metrics::fetch_metrics(kuma_config).await {
        Ok(s) => s,
        Err(e) => return error_result(&format!("Failed to fetch metrics: {e}")),
    };

    // Get all monitor mappings from DB
    let mappings = crate::repo::monitor_repo::list_monitors(&brain.pool)
        .await
        .unwrap_or_default();

    let mut results: Vec<serde_json::Value> = Vec::new();
    for monitor in &summary.monitors {
        // Apply status filter
        if let Some(ref status_filter) = p.status {
            if monitor.status_text != *status_filter {
                continue;
            }
        }

        let mapping = mappings.iter().find(|m| m.monitor_name == monitor.name);
        let mut val = serde_json::to_value(monitor).unwrap_or_default();
        if let Some(mapping) = mapping {
            val["linked_server_id"] = serde_json::to_value(mapping.server_id).unwrap_or_default();
            val["linked_service_id"] = serde_json::to_value(mapping.service_id).unwrap_or_default();
            val["mapping_notes"] = serde_json::to_value(&mapping.notes).unwrap_or_default();
        }
        inject_push_diagnostic_hint(&mut val, monitor);
        results.push(val);
    }

    let output = serde_json::json!({
        "total": summary.total,
        "up": summary.up,
        "down": summary.down,
        "pending": summary.pending,
        "maintenance": summary.maintenance,
        "filtered_count": results.len(),
        "monitors": results,
    });
    json_result(&output)
}

pub(crate) async fn handle_get_monitor_status(
    brain: &super::OpsBrain,
    p: GetMonitorStatusParams,
) -> CallToolResult {
    let kuma_config = match &brain.kuma_config {
        Some(c) => c,
        None => return error_result("Uptime Kuma not configured (set UPTIME_KUMA_URL)"),
    };

    let summary = match crate::metrics::fetch_metrics(kuma_config).await {
        Ok(s) => s,
        Err(e) => return error_result(&format!("Failed to fetch metrics: {e}")),
    };

    let monitor = match summary.monitors.iter().find(|m| m.name == p.monitor_name) {
        Some(m) => m,
        None => return not_found("Monitor", &p.monitor_name),
    };

    let mapping = crate::repo::monitor_repo::get_monitor_by_name(&brain.pool, &p.monitor_name)
        .await
        .ok()
        .flatten();

    let mut result = serde_json::to_value(monitor).unwrap_or_default();
    inject_push_diagnostic_hint(&mut result, monitor);

    // Enrich with linked entities
    if let Some(ref mapping) = mapping {
        if let Some(server_id) = mapping.server_id {
            if let Ok(Some(server)) =
                crate::repo::server_repo::get_server(&brain.pool, server_id).await
            {
                result["linked_server"] = serde_json::to_value(&server).unwrap_or_default();
            }
        }
        if let Some(service_id) = mapping.service_id {
            if let Ok(Some(service)) =
                crate::repo::service_repo::get_service(&brain.pool, service_id).await
            {
                result["linked_service"] = serde_json::to_value(&service).unwrap_or_default();
            }
        }
        result["mapping_notes"] = serde_json::to_value(&mapping.notes).unwrap_or_default();
    }

    json_result(&result)
}

pub(crate) async fn handle_get_monitoring_summary(
    brain: &super::OpsBrain,
    _p: GetMonitoringSummaryParams,
) -> CallToolResult {
    let kuma_config = match &brain.kuma_config {
        Some(c) => c,
        None => return error_result("Uptime Kuma not configured (set UPTIME_KUMA_URL)"),
    };

    let summary = match crate::metrics::fetch_metrics(kuma_config).await {
        Ok(s) => s,
        Err(e) => return error_result(&format!("Failed to fetch metrics: {e}")),
    };

    // Highlight anything that's down, with diagnostic hints for push monitors
    let down_monitors: Vec<serde_json::Value> = summary
        .monitors
        .iter()
        .filter(|m| m.status == 0)
        .map(|m| {
            let mut val = serde_json::to_value(m).unwrap_or_default();
            inject_push_diagnostic_hint(&mut val, m);
            val
        })
        .collect();

    let result = serde_json::json!({
        "status": if summary.down == 0 { "ALL_CLEAR" } else { "DEGRADED" },
        "total": summary.total,
        "up": summary.up,
        "down": summary.down,
        "pending": summary.pending,
        "maintenance": summary.maintenance,
        "down_monitors": down_monitors,
    });
    json_result(&result)
}

pub(crate) async fn handle_link_monitor(
    brain: &super::OpsBrain,
    p: LinkMonitorParams,
) -> CallToolResult {
    // Resolve server slug to ID
    let server_id = match &p.server_slug {
        Some(slug) => match crate::repo::server_repo::get_server_by_slug(&brain.pool, slug).await {
            Ok(Some(s)) => Some(s.id),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Server", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    // Resolve service slug to ID
    let service_id = match &p.service_slug {
        Some(slug) => {
            match crate::repo::service_repo::get_service_by_slug(&brain.pool, slug).await {
                Ok(Some(s)) => Some(s.id),
                Ok(None) => return not_found_with_suggestions(&brain.pool, "Service", slug).await,
                Err(e) => return error_result(&format!("Database error: {e}")),
            }
        }
        None => None,
    };

    if server_id.is_none() && service_id.is_none() && p.notes.is_none() {
        return error_result("Provide at least one of: server_slug, service_slug, or notes");
    }

    match crate::repo::monitor_repo::upsert_monitor(
        &brain.pool,
        &p.monitor_name,
        server_id,
        service_id,
        p.notes.as_deref(),
    )
    .await
    {
        Ok(monitor) => json_result(&monitor),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_unlink_monitor(
    brain: &super::OpsBrain,
    p: UnlinkMonitorParams,
) -> CallToolResult {
    match crate::repo::monitor_repo::delete_monitor(&brain.pool, &p.monitor_name).await {
        Ok(true) => CallToolResult::success(vec![Content::text(format!(
            "Monitor mapping removed: {}",
            p.monitor_name
        ))]),
        Ok(false) => not_found("Monitor mapping", &p.monitor_name),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_list_watchdog_incidents(
    brain: &super::OpsBrain,
    p: ListWatchdogIncidentsParams,
) -> CallToolResult {
    let limit = p.limit.unwrap_or(20);
    let prefix_pattern = format!("{}%", crate::watchdog::INCIDENT_PREFIX);

    let query = match &p.status {
        Some(status) => {
            sqlx::query_as::<_, Incident>(
                "SELECT * FROM incidents WHERE title LIKE $1 AND status = $2 ORDER BY reported_at DESC LIMIT $3",
            )
            .bind(&prefix_pattern)
            .bind(status)
            .bind(limit)
            .fetch_all(&brain.pool)
            .await
        }
        None => {
            sqlx::query_as::<_, Incident>(
                "SELECT * FROM incidents WHERE title LIKE $1 ORDER BY reported_at DESC LIMIT $2",
            )
            .bind(&prefix_pattern)
            .bind(limit)
            .fetch_all(&brain.pool)
            .await
        }
    };

    match query {
        Ok(incidents) => {
            let result = serde_json::json!({
                "count": incidents.len(),
                "incidents": incidents,
            });
            json_result(&result)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}
