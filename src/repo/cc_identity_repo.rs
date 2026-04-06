use sqlx::PgPool;

use crate::models::cc_identity::CcIdentity;

pub async fn get(pool: &PgPool, cc_name: &str) -> Result<Option<CcIdentity>, sqlx::Error> {
    sqlx::query_as::<_, CcIdentity>("SELECT * FROM cc_identities WHERE cc_name = $1")
        .bind(cc_name)
        .fetch_optional(pool)
        .await
}

pub async fn list_all(pool: &PgPool) -> Result<Vec<CcIdentity>, sqlx::Error> {
    sqlx::query_as::<_, CcIdentity>("SELECT * FROM cc_identities ORDER BY cc_name")
        .fetch_all(pool)
        .await
}

/// Internal row shape that joins the identity columns with PostgreSQL's
/// `xmax` system column. `xmax = 0` is true iff the row was newly inserted
/// by THIS statement (vs. modified via `ON CONFLICT DO UPDATE`), giving us
/// race-free first-write detection in a single round trip — no transaction,
/// no SELECT FOR UPDATE (which doesn't lock non-existent rows in PostgreSQL).
#[derive(sqlx::FromRow)]
struct UpsertRow {
    cc_name: String,
    body: String,
    updated_at: chrono::DateTime<chrono::Utc>,
    inserted: bool,
}

/// Upsert an identity row. Returns `(row, was_first_write)` where
/// `was_first_write` is true iff this call inserted the row (vs. updated an
/// existing one). Detection is atomic via `RETURNING (xmax = 0) AS inserted`,
/// so concurrent first-writes for the same `cc_name` cannot both report true.
pub async fn upsert(
    pool: &PgPool,
    cc_name: &str,
    body: &str,
) -> Result<(CcIdentity, bool), sqlx::Error> {
    let row: UpsertRow = sqlx::query_as::<_, UpsertRow>(
        "INSERT INTO cc_identities (cc_name, body, updated_at)
         VALUES ($1, $2, NOW())
         ON CONFLICT (cc_name) DO UPDATE SET
             body = EXCLUDED.body,
             updated_at = NOW()
         RETURNING cc_name, body, updated_at, (xmax = 0) AS inserted",
    )
    .bind(cc_name)
    .bind(body)
    .fetch_one(pool)
    .await?;

    Ok((
        CcIdentity {
            cc_name: row.cc_name,
            body: row.body,
            updated_at: row.updated_at,
        },
        row.inserted,
    ))
}
