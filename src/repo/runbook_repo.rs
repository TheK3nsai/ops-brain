use sqlx::PgPool;
use uuid::Uuid;

use crate::models::runbook::Runbook;

pub async fn get_runbook(pool: &PgPool, id: Uuid) -> Result<Option<Runbook>, sqlx::Error> {
    sqlx::query_as::<_, Runbook>("SELECT * FROM runbooks WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_runbook_by_slug(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<Runbook>, sqlx::Error> {
    sqlx::query_as::<_, Runbook>("SELECT * FROM runbooks WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
}

pub async fn list_runbooks(
    pool: &PgPool,
    category: Option<&str>,
    service_id: Option<Uuid>,
    server_id: Option<Uuid>,
    tag: Option<&str>,
    client_id: Option<Uuid>,
    limit: i64,
) -> Result<Vec<Runbook>, sqlx::Error> {
    let mut query = String::from("SELECT r.* FROM runbooks r");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if service_id.is_some() {
        query.push_str(" JOIN runbook_services rs ON r.id = rs.runbook_id");
        conditions.push(format!("rs.service_id = ${param_idx}"));
        param_idx += 1;
    }
    if server_id.is_some() {
        query.push_str(" JOIN runbook_servers rsv ON r.id = rsv.runbook_id");
        conditions.push(format!("rsv.server_id = ${param_idx}"));
        param_idx += 1;
    }
    if category.is_some() {
        conditions.push(format!("r.category = ${param_idx}"));
        param_idx += 1;
    }
    if tag.is_some() {
        conditions.push(format!("${param_idx} = ANY(r.tags)"));
        param_idx += 1;
    }
    if client_id.is_some() {
        // Show runbooks owned by this client OR global (no client)
        conditions.push(format!(
            "(r.client_id = ${param_idx} OR r.client_id IS NULL)"
        ));
        param_idx += 1;
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY r.title");
    query.push_str(&format!(" LIMIT ${param_idx}"));

    let mut q = sqlx::query_as::<_, Runbook>(&query);
    if let Some(v) = service_id {
        q = q.bind(v);
    }
    if let Some(v) = server_id {
        q = q.bind(v);
    }
    if let Some(v) = category {
        q = q.bind(v);
    }
    if let Some(v) = tag {
        q = q.bind(v);
    }
    if let Some(v) = client_id {
        q = q.bind(v);
    }
    q = q.bind(limit);

    q.fetch_all(pool).await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_runbook(
    pool: &PgPool,
    title: &str,
    slug: &str,
    category: Option<&str>,
    content: &str,
    tags: &[String],
    estimated_minutes: Option<i32>,
    requires_reboot: bool,
    notes: Option<&str>,
    client_id: Option<Uuid>,
    cross_client_safe: bool,
    source_url: Option<&str>,
) -> Result<Runbook, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Runbook>(
        "INSERT INTO runbooks (id, title, slug, category, content, version, tags,
            estimated_minutes, requires_reboot, notes, client_id, cross_client_safe, source_url)
         VALUES ($1, $2, $3, $4, $5, 1, $6, $7, $8, $9, $10, $11, $12)
         RETURNING *",
    )
    .bind(id)
    .bind(title)
    .bind(slug)
    .bind(category)
    .bind(content)
    .bind(tags)
    .bind(estimated_minutes)
    .bind(requires_reboot)
    .bind(notes)
    .bind(client_id)
    .bind(cross_client_safe)
    .bind(source_url)
    .fetch_one(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn update_runbook(
    pool: &PgPool,
    id: Uuid,
    title: Option<&str>,
    category: Option<&str>,
    content: Option<&str>,
    tags: Option<&[String]>,
    estimated_minutes: Option<Option<i32>>,
    requires_reboot: Option<bool>,
    notes: Option<&str>,
    cross_client_safe: Option<bool>,
    source_url: Option<&str>,
) -> Result<Runbook, sqlx::Error> {
    sqlx::query_as::<_, Runbook>(
        "UPDATE runbooks SET
            title = COALESCE($2, title),
            category = COALESCE($3, category),
            content = COALESCE($4, content),
            tags = COALESCE($5, tags),
            estimated_minutes = COALESCE($6, estimated_minutes),
            requires_reboot = COALESCE($7, requires_reboot),
            notes = COALESCE($8, notes),
            cross_client_safe = COALESCE($9, cross_client_safe),
            source_url = COALESCE($10, source_url),
            version = version + 1,
            updated_at = NOW()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(title)
    .bind(category)
    .bind(content)
    .bind(tags)
    .bind(estimated_minutes)
    .bind(requires_reboot)
    .bind(notes)
    .bind(cross_client_safe)
    .bind(source_url)
    .fetch_one(pool)
    .await
}

/// Update last_verified_at timestamp when a runbook is successfully executed.
pub async fn update_last_verified_at(pool: &PgPool, runbook_id: Uuid) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE runbooks SET last_verified_at = now() WHERE id = $1")
        .bind(runbook_id)
        .execute(pool)
        .await?;
    Ok(())
}

/// List runbooks that have never been verified or were last verified before the given threshold.
pub async fn list_stale_runbooks(
    pool: &PgPool,
    stale_days: i32,
    limit: i64,
) -> Result<Vec<Runbook>, sqlx::Error> {
    sqlx::query_as::<_, Runbook>(
        "SELECT * FROM runbooks
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

pub async fn link_runbook_service(
    pool: &PgPool,
    runbook_id: Uuid,
    service_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO runbook_services (runbook_id, service_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(runbook_id)
    .bind(service_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn link_runbook_server(
    pool: &PgPool,
    runbook_id: Uuid,
    server_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO runbook_servers (runbook_id, server_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(runbook_id)
    .bind(server_id)
    .execute(pool)
    .await?;
    Ok(())
}
