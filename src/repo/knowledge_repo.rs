use sqlx::PgPool;
use uuid::Uuid;

use crate::models::knowledge::Knowledge;

pub async fn add_knowledge(
    pool: &PgPool,
    title: &str,
    content: &str,
    category: Option<&str>,
    tags: &[String],
    client_id: Option<Uuid>,
    cross_client_safe: bool,
) -> Result<Knowledge, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Knowledge>(
        "INSERT INTO knowledge (id, title, content, category, tags, client_id, cross_client_safe)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         RETURNING *",
    )
    .bind(id)
    .bind(title)
    .bind(content)
    .bind(category)
    .bind(tags)
    .bind(client_id)
    .bind(cross_client_safe)
    .fetch_one(pool)
    .await
}

pub async fn get_knowledge(pool: &PgPool, id: Uuid) -> Result<Option<Knowledge>, sqlx::Error> {
    sqlx::query_as::<_, Knowledge>("SELECT * FROM knowledge WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list_knowledge(
    pool: &PgPool,
    category: Option<&str>,
    client_id: Option<Uuid>,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    let mut query = String::from("SELECT * FROM knowledge");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if category.is_some() {
        conditions.push(format!("category = ${param_idx}"));
        param_idx += 1;
    }
    if client_id.is_some() {
        conditions.push(format!("client_id = ${param_idx}"));
        let _ = param_idx;
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY title");

    let mut q = sqlx::query_as::<_, Knowledge>(&query);
    if let Some(v) = category {
        q = q.bind(v);
    }
    if let Some(v) = client_id {
        q = q.bind(v);
    }

    q.fetch_all(pool).await
}

pub async fn search_knowledge(pool: &PgPool, query: &str) -> Result<Vec<Knowledge>, sqlx::Error> {
    sqlx::query_as::<_, Knowledge>(
        "SELECT * FROM knowledge
         WHERE search_vector @@ plainto_tsquery('english', $1)
         ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC",
    )
    .bind(query)
    .fetch_all(pool)
    .await
}
