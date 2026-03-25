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

#[allow(clippy::too_many_arguments)]
pub async fn update_knowledge(
    pool: &PgPool,
    id: Uuid,
    title: Option<&str>,
    content: Option<&str>,
    category: Option<&str>,
    tags: Option<&[String]>,
    cross_client_safe: Option<bool>,
) -> Result<Knowledge, sqlx::Error> {
    sqlx::query_as::<_, Knowledge>(
        "UPDATE knowledge SET
            title = COALESCE($2, title),
            content = COALESCE($3, content),
            category = COALESCE($4, category),
            tags = COALESCE($5, tags),
            cross_client_safe = COALESCE($6, cross_client_safe),
            updated_at = NOW()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(title)
    .bind(content)
    .bind(category)
    .bind(tags)
    .bind(cross_client_safe)
    .fetch_one(pool)
    .await
}

pub async fn delete_knowledge(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM knowledge WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
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
