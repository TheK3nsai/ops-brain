use sqlx::PgPool;
use uuid::Uuid;

use crate::models::handoff::Handoff;

pub async fn get_handoff(pool: &PgPool, id: Uuid) -> Result<Option<Handoff>, sqlx::Error> {
    sqlx::query_as::<_, Handoff>("SELECT * FROM handoffs WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_handoff(
    pool: &PgPool,
    from_session_id: Option<Uuid>,
    from_machine: &str,
    to_machine: Option<&str>,
    priority: &str,
    title: &str,
    body: &str,
    context: Option<&serde_json::Value>,
) -> Result<Handoff, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Handoff>(
        "INSERT INTO handoffs (id, from_session_id, from_machine, to_machine, priority, title, body, context)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         RETURNING *",
    )
    .bind(id)
    .bind(from_session_id)
    .bind(from_machine)
    .bind(to_machine)
    .bind(priority)
    .bind(title)
    .bind(body)
    .bind(context)
    .fetch_one(pool)
    .await
}

pub async fn update_handoff_status(
    pool: &PgPool,
    id: Uuid,
    status: &str,
) -> Result<Handoff, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(
        "UPDATE handoffs SET status = $2, updated_at = now()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(status)
    .fetch_one(pool)
    .await
}

pub async fn list_handoffs(
    pool: &PgPool,
    status: Option<&str>,
    to_machine: Option<&str>,
    from_machine: Option<&str>,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    let mut query = String::from("SELECT * FROM handoffs");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if status.is_some() {
        conditions.push(format!("status = ${param_idx}"));
        param_idx += 1;
    }
    if to_machine.is_some() {
        conditions.push(format!("to_machine = ${param_idx}"));
        param_idx += 1;
    }
    if from_machine.is_some() {
        conditions.push(format!("from_machine = ${param_idx}"));
        param_idx += 1;
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY created_at DESC");
    query.push_str(&format!(" LIMIT ${param_idx}"));

    let mut q = sqlx::query_as::<_, Handoff>(&query);
    if let Some(v) = status {
        q = q.bind(v);
    }
    if let Some(v) = to_machine {
        q = q.bind(v);
    }
    if let Some(v) = from_machine {
        q = q.bind(v);
    }
    q = q.bind(limit);

    q.fetch_all(pool).await
}

pub async fn search_handoffs(
    pool: &PgPool,
    query: &str,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(
        "SELECT * FROM handoffs
         WHERE search_vector @@ plainto_tsquery('english', $1)
         ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
         LIMIT $2",
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await
}
