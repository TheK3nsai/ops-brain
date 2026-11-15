//! Read-only queries behind the bus reader (`src/reader`).
//!
//! Operator view. None of these take a `Visibility` party, because the reader
//! shows the whole bus to the person who runs it. Never call them from a path
//! a restricted token can reach.

use chrono::{DateTime, Utc};
use sqlx::PgPool;
use uuid::Uuid;

use super::handoff_repo::HANDOFF_COLS;
use crate::models::handoff::Handoff;

/// A handoff without its body or context: one line in a list.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HandoffLine {
    pub id: Uuid,
    pub from_agent: String,
    pub to_agent: Option<String>,
    pub status: String,
    pub accepted_by: Option<String>,
    pub priority: String,
    pub category: String,
    pub title: String,
    pub origin: String,
    pub repeat_count: i32,
    pub created_at: DateTime<Utc>,
}

/// Where a page ended: the sort key of its last row. Both halves are needed,
/// because two rows can share a timestamp and a cursor on the time alone
/// would skip the second of them.
pub type Cursor = (DateTime<Utc>, Uuid);

/// Every handoff of every status and category, newest first, without bodies.
/// `before` is the `(created_at, id)` of the last row on the previous page.
pub async fn list_lines(
    pool: &PgPool,
    before: Option<Cursor>,
    limit: i64,
) -> Result<Vec<HandoffLine>, sqlx::Error> {
    sqlx::query_as::<_, HandoffLine>(
        "SELECT id, from_agent, to_agent, status, accepted_by, priority, category, title,
                origin, repeat_count, created_at
           FROM handoffs
          WHERE ($1::timestamptz IS NULL OR (created_at, id) < ($1, $2::uuid))
          ORDER BY created_at DESC, id DESC
          LIMIT $3",
    )
    .bind(before.map(|c| c.0))
    .bind(before.map(|c| c.1))
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Most messages one thread page will render. `get_thread` returns up to one
/// more, so the caller can tell a full thread from a cut one.
pub const THREAD_CAP: usize = 500;

/// The whole thread a handoff belongs to, oldest first: walk `in_reply_to` up
/// to the root, then collect every descendant of that root. Empty when `id`
/// does not exist. A reply whose parent was deleted has a NULL `in_reply_to`
/// (the foreign key sets it), so it is the root of its own thread.
pub async fn get_thread(pool: &PgPool, id: Uuid) -> Result<Vec<Handoff>, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(&format!(
        "WITH RECURSIVE up AS (
             SELECT id, in_reply_to, 0 AS depth FROM handoffs WHERE id = $1
             UNION ALL
             SELECT h.id, h.in_reply_to, up.depth + 1
               FROM handoffs h JOIN up ON h.id = up.in_reply_to
              WHERE up.depth < 1000
         ),
         down AS (
             SELECT root.id, 0 AS depth
               FROM (SELECT id FROM up ORDER BY depth DESC LIMIT 1) root
             UNION ALL
             SELECT h.id, down.depth + 1
               FROM handoffs h JOIN down ON h.in_reply_to = down.id
              WHERE down.depth < 1000
         )
         SELECT {HANDOFF_COLS} FROM handoffs
          WHERE id IN (SELECT id FROM down)
          ORDER BY created_at ASC, id ASC
          LIMIT $2"
    ))
    .bind(id)
    .bind(THREAD_CAP as i64 + 1)
    .fetch_all(pool)
    .await
}

/// The bodies of a few handoffs by id, for snippets on rows that were listed
/// without them.
pub async fn bodies(pool: &PgPool, ids: &[Uuid]) -> Result<Vec<(Uuid, String)>, sqlx::Error> {
    sqlx::query_as("SELECT id, body FROM handoffs WHERE id = ANY($1)")
        .bind(ids)
        .fetch_all(pool)
        .await
}

/// One conversation: a root handoff and the replies under it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ThreadLine {
    pub root: Uuid,
    pub title: String,
    pub from_agent: String,
    pub to_agent: Option<String>,
    pub messages: i64,
    /// Action handoffs in the thread that are still pending or accepted.
    pub open: i64,
    pub last_at: DateTime<Utc>,
    pub senders: Vec<String>,
}

/// Threads with at least one reply, most recent activity first. `before` is
/// the `(last_at, root)` of the last row on the previous page.
///
/// The recursion labels every handoff with its root on each call. That is a
/// scan of the table, which is the right trade for a page one person opens.
pub async fn list_threads(
    pool: &PgPool,
    before: Option<Cursor>,
    limit: i64,
) -> Result<Vec<ThreadLine>, sqlx::Error> {
    sqlx::query_as::<_, ThreadLine>(
        "WITH RECURSIVE t AS (
             SELECT id, id AS root FROM handoffs WHERE in_reply_to IS NULL
             UNION ALL
             SELECT h.id, t.root FROM handoffs h JOIN t ON h.in_reply_to = t.id
         ),
         g AS (
             SELECT t.root,
                    count(*) AS messages,
                    count(*) FILTER (WHERE h.category = 'action'
                                       AND h.status IN ('pending', 'accepted')) AS open,
                    max(h.created_at) AS last_at,
                    array_agg(DISTINCT h.from_agent::text) AS senders
               FROM t JOIN handoffs h ON h.id = t.id
              GROUP BY t.root
             HAVING count(*) > 1
         )
         SELECT g.root, r.title, r.from_agent, r.to_agent, g.messages, g.open, g.last_at,
                g.senders
           FROM g JOIN handoffs r ON r.id = g.root
          WHERE ($1::timestamptz IS NULL OR (g.last_at, g.root) < ($1, $2::uuid))
          ORDER BY g.last_at DESC, g.root DESC
          LIMIT $3",
    )
    .bind(before.map(|c| c.0))
    .bind(before.map(|c| c.1))
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// One slug as the handoffs show it. There is no registry behind this: a slug
/// exists here because a handoff names it.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AgentLine {
    /// Most recent spelling; slugs compare case-insensitively.
    pub name: String,
    pub filed: i64,
    pub received: i64,
    /// Open action handoffs addressed to this slug, leaving out the accepted
    /// self-addressed claims that are locks rather than work. An unaddressed
    /// action somebody accepted is not counted here, though the open-work page
    /// lists it under its acceptor; only rows older than v7.0.0 can be one.
    pub owes: i64,
    pub last_filed: Option<DateTime<Utc>>,
    /// Every handoff it filed came through the machine ingestion path. `None`
    /// when it has filed nothing.
    pub machine_only: Option<bool>,
}

/// Every slug on the bus in order of first appearance, so a slug keeps its
/// position (and the colour the reader derives from it) as others arrive.
pub async fn agent_roster(pool: &PgPool) -> Result<Vec<AgentLine>, sqlx::Error> {
    sqlx::query_as::<_, AgentLine>(
        "SELECT (array_agg(name ORDER BY created_at DESC))[1] AS name,
                count(*) FILTER (WHERE sent) AS filed,
                count(*) FILTER (WHERE NOT sent) AS received,
                count(*) FILTER (WHERE owed) AS owes,
                max(created_at) FILTER (WHERE sent) AS last_filed,
                bool_and(origin = 'machine') FILTER (WHERE sent) AS machine_only
           FROM (
                SELECT from_agent::text AS name, true AS sent, false AS owed,
                       created_at, origin
                  FROM handoffs
                UNION ALL
                SELECT to_agent::text, false,
                       category = 'action' AND status IN ('pending', 'accepted')
                         AND NOT (status = 'accepted'
                                  AND LOWER(to_agent) = LOWER(from_agent)),
                       created_at, origin
                  FROM handoffs
                 WHERE to_agent IS NOT NULL
           ) x
          GROUP BY LOWER(name)
          ORDER BY min(created_at) ASC, LOWER(name) ASC",
    )
    .fetch_all(pool)
    .await
}
