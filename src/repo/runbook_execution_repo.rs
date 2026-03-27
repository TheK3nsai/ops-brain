use sqlx::PgPool;
use uuid::Uuid;

use crate::models::runbook_execution::RunbookExecution;

#[allow(clippy::too_many_arguments)]
pub async fn log_execution(
    pool: &PgPool,
    runbook_id: Uuid,
    executor: &str,
    result: &str,
    notes: Option<&str>,
    duration_minutes: Option<i32>,
    executed_at: Option<chrono::DateTime<chrono::Utc>>,
    client_slug: Option<&str>,
    incident_id: Option<Uuid>,
) -> Result<RunbookExecution, sqlx::Error> {
    let now = executed_at.unwrap_or_else(chrono::Utc::now);
    sqlx::query_as::<_, RunbookExecution>(
        "INSERT INTO runbook_executions (id, runbook_id, executor, result, notes, duration_minutes, executed_at, client_slug, incident_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING *",
    )
    .bind(Uuid::now_v7())
    .bind(runbook_id)
    .bind(executor)
    .bind(result)
    .bind(notes)
    .bind(duration_minutes)
    .bind(now)
    .bind(client_slug)
    .bind(incident_id)
    .fetch_one(pool)
    .await
}

/// List executions linked to a specific incident.
pub async fn list_executions_for_incident(
    pool: &PgPool,
    incident_id: Uuid,
    limit: i64,
) -> Result<Vec<RunbookExecution>, sqlx::Error> {
    sqlx::query_as::<_, RunbookExecution>(
        "SELECT * FROM runbook_executions WHERE incident_id = $1 ORDER BY executed_at DESC LIMIT $2",
    )
    .bind(incident_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn list_executions_for_runbook(
    pool: &PgPool,
    runbook_id: Uuid,
    limit: i64,
) -> Result<Vec<RunbookExecution>, sqlx::Error> {
    sqlx::query_as::<_, RunbookExecution>(
        "SELECT * FROM runbook_executions WHERE runbook_id = $1 ORDER BY executed_at DESC LIMIT $2",
    )
    .bind(runbook_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn list_recent_executions(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<RunbookExecution>, sqlx::Error> {
    sqlx::query_as::<_, RunbookExecution>(
        "SELECT * FROM runbook_executions ORDER BY executed_at DESC LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}
