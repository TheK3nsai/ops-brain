use sqlx::PgPool;
use uuid::Uuid;

use crate::models::briefing::Briefing;

pub async fn insert_briefing(
    pool: &PgPool,
    briefing_type: &str,
    client_id: Option<Uuid>,
    content: &str,
) -> Result<Briefing, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Briefing>(
        "INSERT INTO briefings (id, briefing_type, client_id, content)
         VALUES ($1, $2, $3, $4)
         RETURNING *",
    )
    .bind(id)
    .bind(briefing_type)
    .bind(client_id)
    .bind(content)
    .fetch_one(pool)
    .await
}

pub async fn list_briefings(
    pool: &PgPool,
    briefing_type: Option<&str>,
    client_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Briefing>, sqlx::Error> {
    let mut query = String::from("SELECT * FROM briefings");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if briefing_type.is_some() {
        conditions.push(format!("briefing_type = ${param_idx}"));
        param_idx += 1;
    }
    if client_id.is_some() {
        conditions.push(format!("client_id = ${param_idx}"));
        param_idx += 1;
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY generated_at DESC");
    query.push_str(&format!(" LIMIT ${param_idx}"));

    let mut q = sqlx::query_as::<_, Briefing>(&query);
    if let Some(v) = briefing_type {
        q = q.bind(v);
    }
    if let Some(v) = client_id {
        q = q.bind(v);
    }
    q = q.bind(limit);

    q.fetch_all(pool).await
}

pub async fn get_briefing(pool: &PgPool, id: Uuid) -> Result<Option<Briefing>, sqlx::Error> {
    sqlx::query_as::<_, Briefing>("SELECT * FROM briefings WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}
