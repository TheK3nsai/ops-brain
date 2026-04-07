use sqlx::PgPool;
use uuid::Uuid;

use crate::models::knowledge::Knowledge;

#[allow(clippy::too_many_arguments)]
pub async fn add_knowledge(
    pool: &PgPool,
    title: &str,
    content: &str,
    category: Option<&str>,
    tags: &[String],
    client_id: Option<Uuid>,
    cross_client_safe: bool,
    author_cc: Option<&str>,
    source_incident_id: Option<Uuid>,
) -> Result<Knowledge, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Knowledge>(
        "INSERT INTO knowledge (id, title, content, category, tags, client_id, cross_client_safe, author_cc, source_incident_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING *",
    )
    .bind(id)
    .bind(title)
    .bind(content)
    .bind(category)
    .bind(tags)
    .bind(client_id)
    .bind(cross_client_safe)
    .bind(author_cc)
    .bind(source_incident_id)
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
    limit: i64,
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
        param_idx += 1;
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY title");
    query.push_str(&format!(" LIMIT ${param_idx}"));

    let mut q = sqlx::query_as::<_, Knowledge>(&query);
    if let Some(v) = category {
        q = q.bind(v);
    }
    if let Some(v) = client_id {
        q = q.bind(v);
    }
    q = q.bind(limit);

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
    source_incident_id: Option<Uuid>,
) -> Result<Knowledge, sqlx::Error> {
    // NOTE: `author_cc` is intentionally NOT updatable — provenance is
    // immutable via the tool surface. Direct SQL is still possible for
    // emergency correction. See migration 20260408000001.
    sqlx::query_as::<_, Knowledge>(
        "UPDATE knowledge SET
            title = COALESCE($2, title),
            content = COALESCE($3, content),
            category = COALESCE($4, category),
            tags = COALESCE($5, tags),
            cross_client_safe = COALESCE($6, cross_client_safe),
            source_incident_id = COALESCE($7, source_incident_id),
            updated_at = NOW()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(title)
    .bind(content)
    .bind(category)
    .bind(tags)
    .bind(cross_client_safe)
    .bind(source_incident_id)
    .fetch_one(pool)
    .await
}

/// Mark a knowledge entry as verified (confirms content is still accurate).
pub async fn update_last_verified_at(pool: &PgPool, id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE knowledge SET last_verified_at = now() WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

/// List knowledge entries never verified or last verified before the given threshold.
pub async fn list_stale_knowledge(
    pool: &PgPool,
    stale_days: i32,
    limit: i64,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    sqlx::query_as::<_, Knowledge>(
        "SELECT * FROM knowledge
         WHERE last_verified_at IS NULL
            OR last_verified_at < now() - ($1 || ' days')::interval
         ORDER BY last_verified_at ASC NULLS FIRST
         LIMIT $2",
    )
    .bind(stale_days)
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn delete_knowledge(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM knowledge WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn search_knowledge(
    pool: &PgPool,
    query: &str,
    limit: i64,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    let results = sqlx::query_as::<_, Knowledge>(
        "SELECT * FROM knowledge
         WHERE search_vector @@ websearch_to_tsquery('english', $1)
         ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
         LIMIT $2",
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    if results.is_empty() {
        if let Some(or_text) = super::build_or_tsquery_text(query) {
            return sqlx::query_as::<_, Knowledge>(
                "SELECT * FROM knowledge
                 WHERE search_vector @@ to_tsquery('english', $1)
                 ORDER BY ts_rank(search_vector, to_tsquery('english', $1)) DESC
                 LIMIT $2",
            )
            .bind(&or_text)
            .bind(limit)
            .fetch_all(pool)
            .await;
        }
    }

    Ok(results)
}
