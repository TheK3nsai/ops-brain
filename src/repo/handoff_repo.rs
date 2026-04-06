use sqlx::PgPool;
use uuid::Uuid;

use crate::models::handoff::Handoff;

/// How long a notify-class handoff stays visible in operational queries
/// (list_handoffs, check_in) before being filtered out at read time. The row
/// itself is preserved for audit and search history.
pub const NOTIFY_TTL_DAYS: i32 = 7;

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
    category: &str,
    title: &str,
    body: &str,
    context: Option<&serde_json::Value>,
) -> Result<Handoff, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Handoff>(
        "INSERT INTO handoffs (id, from_session_id, from_machine, to_machine, priority, category, title, body, context)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING *",
    )
    .bind(id)
    .bind(from_session_id)
    .bind(from_machine)
    .bind(to_machine)
    .bind(priority)
    .bind(category)
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

#[allow(clippy::too_many_arguments)]
pub async fn list_handoffs(
    pool: &PgPool,
    status: Option<&str>,
    to_machine: Option<&str>,
    from_machine: Option<&str>,
    category: Option<&str>,
    include_notify: bool,
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
    if category.is_some() {
        // Explicit category filter wins; include_notify is ignored.
        conditions.push(format!("category = ${param_idx}"));
        param_idx += 1;
    } else if !include_notify {
        // Default: action queue only.
        conditions.push("category = 'action'".to_string());
    }

    // Read-time pruning: stale notify rows never resurface in operational
    // queries. The row stays in the table; it just stops being noise.
    conditions.push(format!(
        "(category = 'action' OR created_at > now() - interval '{NOTIFY_TTL_DAYS} days')"
    ));

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
    if let Some(v) = category {
        q = q.bind(v);
    }
    q = q.bind(limit);

    q.fetch_all(pool).await
}

pub async fn delete_handoff(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM handoffs WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
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
