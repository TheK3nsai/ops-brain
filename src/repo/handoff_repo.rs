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
    from_agent: &str,
    to_agent: Option<&str>,
    priority: &str,
    category: &str,
    title: &str,
    body: &str,
    context: Option<&serde_json::Value>,
    in_reply_to: Option<Uuid>,
) -> Result<Handoff, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Handoff>(
        "INSERT INTO handoffs (id, from_agent, to_agent, priority, category, title, body, context, in_reply_to)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
         RETURNING *",
    )
    .bind(id)
    .bind(from_agent)
    .bind(to_agent)
    .bind(priority)
    .bind(category)
    .bind(title)
    .bind(body)
    .bind(context)
    .bind(in_reply_to)
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

/// Complete a handoff, optionally recording the commit hash of the work
/// that closed it. `commit_hash` is free-form; callers typically pass a
/// short or full git SHA, but any opaque identifier works.
pub async fn complete_handoff_with_commit(
    pool: &PgPool,
    id: Uuid,
    commit_hash: Option<&str>,
) -> Result<Handoff, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(
        "UPDATE handoffs
            SET status = 'completed',
                commit_hash = COALESCE($2, commit_hash),
                updated_at = now()
          WHERE id = $1
          RETURNING *",
    )
    .bind(id)
    .bind(commit_hash)
    .fetch_one(pool)
    .await
}

/// Mark a handoff as merged. Records the merge commit (typically the
/// merge-to-main commit that bundled the work) and the merge timestamp.
/// Does NOT require `commit_hash` to be set; callers who care about that
/// linkage can verify it client-side before invoking.
pub async fn mark_merged(
    pool: &PgPool,
    id: Uuid,
    merge_commit: &str,
) -> Result<Handoff, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(
        "UPDATE handoffs
            SET status = 'merged',
                merge_commit = $2,
                merged_at = now(),
                updated_at = now()
          WHERE id = $1
          RETURNING *",
    )
    .bind(id)
    .bind(merge_commit)
    .fetch_one(pool)
    .await
}

/// Replies addressed to `agent`: handoffs whose `in_reply_to` references a
/// handoff that `agent` originally sent. Optional `since` filters by the
/// reply's `created_at`.
pub async fn list_replies_to_me(
    pool: &PgPool,
    agent: &str,
    since: Option<chrono::DateTime<chrono::Utc>>,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    let mut q = String::from(
        "SELECT r.*
           FROM handoffs r
           JOIN handoffs parent ON parent.id = r.in_reply_to
          WHERE parent.from_agent ILIKE $1",
    );
    if since.is_some() {
        q.push_str(" AND r.created_at > $2");
        q.push_str(" ORDER BY r.created_at DESC LIMIT $3");
    } else {
        q.push_str(" ORDER BY r.created_at DESC LIMIT $2");
    }

    let mut query = sqlx::query_as::<_, Handoff>(&q).bind(agent);
    if let Some(ts) = since {
        query = query.bind(ts);
    }
    query = query.bind(limit);
    query.fetch_all(pool).await
}

#[allow(clippy::too_many_arguments)]
pub async fn list_handoffs(
    pool: &PgPool,
    status: Option<&str>,
    to_agent: Option<&str>,
    from_agent: Option<&str>,
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
    if to_agent.is_some() {
        conditions.push(format!("to_agent ILIKE ${param_idx}"));
        param_idx += 1;
    }
    if from_agent.is_some() {
        conditions.push(format!("from_agent ILIKE ${param_idx}"));
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
    if let Some(v) = to_agent {
        q = q.bind(v);
    }
    if let Some(v) = from_agent {
        q = q.bind(v);
    }
    if let Some(v) = category {
        q = q.bind(v);
    }
    q = q.bind(limit);

    q.fetch_all(pool).await
}

pub async fn list_open_handoffs(
    pool: &PgPool,
    to_agent: Option<&str>,
    from_agent: Option<&str>,
    category: Option<&str>,
    include_notify: bool,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    let mut query = String::from("SELECT * FROM handoffs");
    let mut conditions: Vec<String> = vec!["status IN ('pending', 'accepted')".to_string()];
    let mut param_idx = 1u32;

    if to_agent.is_some() {
        conditions.push(format!("to_agent ILIKE ${param_idx}"));
        param_idx += 1;
    }
    if from_agent.is_some() {
        conditions.push(format!("from_agent ILIKE ${param_idx}"));
        param_idx += 1;
    }
    if category.is_some() {
        conditions.push(format!("category = ${param_idx}"));
        param_idx += 1;
    } else if !include_notify {
        conditions.push("category = 'action'".to_string());
    }

    conditions.push(format!(
        "(category = 'action' OR created_at > now() - interval '{NOTIFY_TTL_DAYS} days')"
    ));

    query.push_str(" WHERE ");
    query.push_str(&conditions.join(" AND "));
    query.push_str(" ORDER BY created_at DESC");
    query.push_str(&format!(" LIMIT ${param_idx}"));

    let mut q = sqlx::query_as::<_, Handoff>(&query);
    if let Some(v) = to_agent {
        q = q.bind(v);
    }
    if let Some(v) = from_agent {
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
