use sqlx::PgPool;
use uuid::Uuid;

use crate::models::handoff::Handoff;

/// Explicit column list for `Handoff` reads — matches the model fields exactly
/// and deliberately omits `embedding` (768-dim vector, ~3KB/row) and
/// `search_vector`, which `FromRow` discards anyway. Use this instead of
/// `SELECT *` (or `SELECT h.*` via `aliased_cols`) so vectors never cross the
/// wire on read paths. Writes keep `RETURNING *`.
pub const HANDOFF_COLS: &str =
    "id, from_agent, to_agent, status, priority, category, title, body, context, \
     in_reply_to, commit_hash, merge_commit, merged_at, origin, dedupe_key, \
     repeat_count, created_at, updated_at";

/// How long a notify-class handoff stays visible in operational queries
/// (list_handoffs, check_in) before being filtered out at read time. The row
/// itself is preserved for audit and search history.
pub const NOTIFY_TTL_DAYS: i32 = 7;

pub async fn get_handoff(pool: &PgPool, id: Uuid) -> Result<Option<Handoff>, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(&format!(
        "SELECT {HANDOFF_COLS} FROM handoffs WHERE id = $1"
    ))
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

/// Insert a machine-filed handoff (`origin = 'machine'`), idempotent on
/// `(dedupe_key, recipient)` against OPEN rows: if a pending/accepted
/// handoff to the same agent already holds the key, the insert collapses
/// into a bump of that row's `repeat_count` + `updated_at` and returns it
/// unchanged otherwise. The same key filed to a *different* agent inserts
/// independently — dedupe scope is per recipient, not global.
///
/// The ON CONFLICT clause must stay in sync with the partial unique index
/// `idx_handoffs_dedupe_key_open` (see migration 20260717220000).
#[allow(clippy::too_many_arguments)]
pub async fn create_machine_handoff(
    pool: &PgPool,
    from_agent: &str,
    to_agent: &str,
    priority: &str,
    category: &str,
    title: &str,
    body: &str,
    context: Option<&serde_json::Value>,
    dedupe_key: Option<&str>,
) -> Result<Handoff, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Handoff>(
        "INSERT INTO handoffs (id, from_agent, to_agent, priority, category, title, body,
                               context, origin, dedupe_key)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, 'machine', $9)
         ON CONFLICT (dedupe_key, LOWER(to_agent))
            WHERE dedupe_key IS NOT NULL AND status IN ('pending', 'accepted')
         DO UPDATE SET repeat_count = handoffs.repeat_count + 1,
                       updated_at = now()
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
    .bind(dedupe_key)
    .fetch_one(pool)
    .await
}

/// Open action handoffs addressed to `agent`, for wake-shim polling.
/// `since` filters on `updated_at` (which `create_machine_handoff` bumps on
/// dedupe suppression, so a still-firing monitor re-surfaces past a cursor).
pub async fn list_pending_for_agent(
    pool: &PgPool,
    agent: &str,
    since: Option<chrono::DateTime<chrono::Utc>>,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    // Exact case-insensitive match, NOT ILIKE: this query sits behind the
    // machine-token agent allowlist (an exact compare), and ILIKE's `_`
    // wildcard — legal in agent names — would over-match past that gate.
    let mut q = format!(
        "SELECT {HANDOFF_COLS} FROM handoffs
          WHERE status IN ('pending', 'accepted')
            AND category = 'action'
            AND LOWER(to_agent) = LOWER($1)"
    );
    if since.is_some() {
        q.push_str(" AND updated_at > $2 ORDER BY updated_at DESC LIMIT $3");
    } else {
        q.push_str(" ORDER BY updated_at DESC LIMIT $2");
    }

    let mut query = sqlx::query_as::<_, Handoff>(&q).bind(agent);
    if let Some(ts) = since {
        query = query.bind(ts);
    }
    query = query.bind(limit);
    query.fetch_all(pool).await
}

/// Atomically accept a pending handoff. The status precondition lives in
/// the UPDATE itself so two agents racing on the same handoff can't both
/// win — the loser gets `None` and must re-read to see who beat them.
pub async fn accept_handoff(pool: &PgPool, id: Uuid) -> Result<Option<Handoff>, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(
        "UPDATE handoffs SET status = 'accepted', updated_at = now()
         WHERE id = $1 AND status = 'pending'
         RETURNING *",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

/// Complete a handoff, optionally recording the commit hash of the work
/// that closed it. `commit_hash` is free-form; callers typically pass a
/// short or full git SHA, but any opaque identifier works.
///
/// Atomic: only open (pending/accepted) rows transition. `None` means the
/// row is missing or already terminal — callers re-read to report which.
pub async fn complete_handoff_with_commit(
    pool: &PgPool,
    id: Uuid,
    commit_hash: Option<&str>,
) -> Result<Option<Handoff>, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(
        "UPDATE handoffs
            SET status = 'completed',
                commit_hash = COALESCE($2, commit_hash),
                updated_at = now()
          WHERE id = $1 AND status IN ('pending', 'accepted')
          RETURNING *",
    )
    .bind(id)
    .bind(commit_hash)
    .fetch_optional(pool)
    .await
}

/// Mark a handoff as merged. Records the merge commit (typically the
/// merge-to-main commit that bundled the work) and the merge timestamp.
/// Does NOT require `commit_hash` to be set; callers who care about that
/// linkage can verify it client-side before invoking.
///
/// Atomic: only 'completed' rows transition, so a concurrent merge (or a
/// re-open) between the caller's precondition read and this write loses
/// cleanly with `None` instead of overwriting.
pub async fn mark_merged(
    pool: &PgPool,
    id: Uuid,
    merge_commit: &str,
) -> Result<Option<Handoff>, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(
        "UPDATE handoffs
            SET status = 'merged',
                merge_commit = $2,
                merged_at = now(),
                updated_at = now()
          WHERE id = $1 AND status = 'completed'
          RETURNING *",
    )
    .bind(id)
    .bind(merge_commit)
    .fetch_optional(pool)
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
    let mut q = format!(
        "SELECT {}
           FROM handoffs r
           JOIN handoffs parent ON parent.id = r.in_reply_to
          WHERE LOWER(parent.from_agent) = LOWER($1)",
        super::aliased_cols(HANDOFF_COLS, "r")
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

/// How the status column is constrained in `list_handoffs_filtered`.
#[derive(Clone, Copy)]
enum StatusFilter<'a> {
    /// No status constraint (list_handoffs with no status filter).
    Any,
    /// Exactly one status (`status = $n`, bound).
    Exact(&'a str),
    /// The open set — pending + accepted (list_open_handoffs).
    Open,
}

/// Shared builder behind `list_handoffs` and `list_open_handoffs`. The only
/// axis that differs between the two public entry points is how the status
/// column is constrained (`status_filter`); agent/category filtering, the
/// notify-TTL read-time pruning clause, ordering, and the limit are identical.
/// Keeping this in one place means the NOTIFY_TTL pruning lives in exactly one
/// spot.
async fn list_handoffs_filtered(
    pool: &PgPool,
    status_filter: StatusFilter<'_>,
    to_agent: Option<&str>,
    from_agent: Option<&str>,
    category: Option<&str>,
    include_notify: bool,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    let mut query = format!("SELECT {HANDOFF_COLS} FROM handoffs");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    match status_filter {
        StatusFilter::Any => {}
        StatusFilter::Exact(_) => {
            conditions.push(format!("status = ${param_idx}"));
            param_idx += 1;
        }
        StatusFilter::Open => {
            conditions.push("status IN ('pending', 'accepted')".to_string());
        }
    }

    // Exact case-insensitive agent matching, NOT ILIKE: `_` is legal in
    // agent names and ILIKE treats it as a single-char wildcard (same
    // reasoning as list_pending_for_agent).
    if to_agent.is_some() {
        conditions.push(format!("LOWER(to_agent) = LOWER(${param_idx})"));
        param_idx += 1;
    }
    if from_agent.is_some() {
        conditions.push(format!("LOWER(from_agent) = LOWER(${param_idx})"));
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
    // queries. The row stays in the table; it just stops being noise. This is
    // the single source of the NOTIFY_TTL clause.
    conditions.push(format!(
        "(category = 'action' OR created_at > now() - interval '{NOTIFY_TTL_DAYS} days')"
    ));

    query.push_str(" WHERE ");
    query.push_str(&conditions.join(" AND "));
    query.push_str(" ORDER BY created_at DESC");
    query.push_str(&format!(" LIMIT ${param_idx}"));

    let mut q = sqlx::query_as::<_, Handoff>(&query);
    if let StatusFilter::Exact(s) = status_filter {
        q = q.bind(s);
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
    let status_filter = match status {
        Some(s) => StatusFilter::Exact(s),
        None => StatusFilter::Any,
    };
    list_handoffs_filtered(
        pool,
        status_filter,
        to_agent,
        from_agent,
        category,
        include_notify,
        limit,
    )
    .await
}

pub async fn list_open_handoffs(
    pool: &PgPool,
    to_agent: Option<&str>,
    from_agent: Option<&str>,
    category: Option<&str>,
    include_notify: bool,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    list_handoffs_filtered(
        pool,
        StatusFilter::Open,
        to_agent,
        from_agent,
        category,
        include_notify,
        limit,
    )
    .await
}

/// Real open-action-handoff counts for briefings. Mirrors the filter of
/// `list_open_handoffs(.., include_notify=false)` (action-class, pending +
/// accepted) but returns true totals instead of a `LIMIT`-bounded page — the
/// briefing previously reported `len()` of a 20-row page as the open count.
pub struct OpenHandoffCounts {
    pub open: i64,
    pub pending: i64,
    pub accepted: i64,
}

pub async fn count_open_handoffs(pool: &PgPool) -> Result<OpenHandoffCounts, sqlx::Error> {
    let row: (i64, i64, i64) = sqlx::query_as(
        "SELECT
            count(*),
            count(*) FILTER (WHERE status = 'pending'),
            count(*) FILTER (WHERE status = 'accepted')
         FROM handoffs
         WHERE category = 'action' AND status IN ('pending', 'accepted')",
    )
    .fetch_one(pool)
    .await?;
    Ok(OpenHandoffCounts {
        open: row.0,
        pending: row.1,
        accepted: row.2,
    })
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
    sqlx::query_as::<_, Handoff>(&format!(
        "SELECT {HANDOFF_COLS} FROM handoffs
         WHERE search_vector @@ plainto_tsquery('english', $1)
         ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
         LIMIT $2"
    ))
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await
}
