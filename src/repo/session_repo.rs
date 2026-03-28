use sqlx::PgPool;
use uuid::Uuid;

use crate::models::session::Session;

pub async fn get_session(pool: &PgPool, id: Uuid) -> Result<Option<Session>, sqlx::Error> {
    sqlx::query_as::<_, Session>("SELECT * FROM sessions WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn start_session(
    pool: &PgPool,
    machine_id: &str,
    machine_hostname: &str,
) -> Result<Session, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Session>(
        "INSERT INTO sessions (id, machine_id, machine_hostname)
         VALUES ($1, $2, $3)
         RETURNING *",
    )
    .bind(id)
    .bind(machine_id)
    .bind(machine_hostname)
    .fetch_one(pool)
    .await
}

pub async fn end_session(
    pool: &PgPool,
    id: Uuid,
    summary: Option<&str>,
) -> Result<Session, sqlx::Error> {
    sqlx::query_as::<_, Session>(
        "UPDATE sessions SET
            ended_at = now(),
            summary = COALESCE($2, summary)
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(summary)
    .fetch_one(pool)
    .await
}

pub async fn list_sessions(
    pool: &PgPool,
    machine_id: Option<&str>,
    active_only: bool,
    limit: i64,
) -> Result<Vec<Session>, sqlx::Error> {
    let mut query = String::from("SELECT * FROM sessions");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if machine_id.is_some() {
        conditions.push(format!("machine_id = ${param_idx}"));
        param_idx += 1;
    }
    if active_only {
        conditions.push("ended_at IS NULL".to_string());
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY started_at DESC");
    query.push_str(&format!(" LIMIT ${param_idx}"));

    let mut q = sqlx::query_as::<_, Session>(&query);
    if let Some(v) = machine_id {
        q = q.bind(v);
    }
    q = q.bind(limit);

    q.fetch_all(pool).await
}
