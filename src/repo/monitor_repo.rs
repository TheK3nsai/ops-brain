use sqlx::PgPool;
use uuid::Uuid;

use crate::models::monitor::Monitor;

pub async fn upsert_monitor(
    pool: &PgPool,
    monitor_name: &str,
    server_id: Option<Uuid>,
    service_id: Option<Uuid>,
    notes: Option<&str>,
    severity_override: Option<&str>,
    flap_threshold: Option<i32>,
) -> Result<Monitor, sqlx::Error> {
    sqlx::query_as::<_, Monitor>(
        "INSERT INTO monitors (id, monitor_name, server_id, service_id, notes, severity_override, flap_threshold)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (monitor_name) DO UPDATE SET
             server_id = COALESCE(EXCLUDED.server_id, monitors.server_id),
             service_id = COALESCE(EXCLUDED.service_id, monitors.service_id),
             notes = COALESCE(EXCLUDED.notes, monitors.notes),
             severity_override = COALESCE(EXCLUDED.severity_override, monitors.severity_override),
             flap_threshold = COALESCE(EXCLUDED.flap_threshold, monitors.flap_threshold),
             updated_at = now()
         RETURNING *",
    )
    .bind(Uuid::now_v7())
    .bind(monitor_name)
    .bind(server_id)
    .bind(service_id)
    .bind(notes)
    .bind(severity_override)
    .bind(flap_threshold)
    .fetch_one(pool)
    .await
}

pub async fn get_monitor_by_name(
    pool: &PgPool,
    monitor_name: &str,
) -> Result<Option<Monitor>, sqlx::Error> {
    sqlx::query_as::<_, Monitor>("SELECT * FROM monitors WHERE monitor_name = $1")
        .bind(monitor_name)
        .fetch_optional(pool)
        .await
}

pub async fn list_monitors(pool: &PgPool) -> Result<Vec<Monitor>, sqlx::Error> {
    sqlx::query_as::<_, Monitor>("SELECT * FROM monitors ORDER BY monitor_name")
        .fetch_all(pool)
        .await
}

pub async fn get_monitors_for_server(
    pool: &PgPool,
    server_id: Uuid,
) -> Result<Vec<Monitor>, sqlx::Error> {
    sqlx::query_as::<_, Monitor>(
        "SELECT * FROM monitors WHERE server_id = $1 ORDER BY monitor_name",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn get_monitors_for_service(
    pool: &PgPool,
    service_id: Uuid,
) -> Result<Vec<Monitor>, sqlx::Error> {
    sqlx::query_as::<_, Monitor>(
        "SELECT * FROM monitors WHERE service_id = $1 ORDER BY monitor_name",
    )
    .bind(service_id)
    .fetch_all(pool)
    .await
}

pub async fn delete_monitor(pool: &PgPool, monitor_name: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM monitors WHERE monitor_name = $1")
        .bind(monitor_name)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
