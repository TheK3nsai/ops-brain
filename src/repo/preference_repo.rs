use sqlx::PgPool;

use crate::models::preference::Preference;

/// Get a preference by scope and key.
pub async fn get_preference(
    pool: &PgPool,
    scope: &str,
    key: &str,
) -> Result<Option<Preference>, sqlx::Error> {
    sqlx::query_as::<_, Preference>("SELECT * FROM preferences WHERE scope = $1 AND key = $2")
        .bind(scope)
        .bind(key)
        .fetch_optional(pool)
        .await
}

/// Set a preference (upsert). Returns the upserted row.
pub async fn set_preference(
    pool: &PgPool,
    scope: &str,
    key: &str,
    value: &serde_json::Value,
) -> Result<Preference, sqlx::Error> {
    sqlx::query_as::<_, Preference>(
        "INSERT INTO preferences (scope, key, value)
         VALUES ($1, $2, $3)
         ON CONFLICT (scope, key) DO UPDATE SET value = $3, updated_at = now()
         RETURNING *",
    )
    .bind(scope)
    .bind(key)
    .bind(value)
    .fetch_one(pool)
    .await
}

/// List all preferences, optionally filtered by scope.
pub async fn list_preferences(
    pool: &PgPool,
    scope: Option<&str>,
) -> Result<Vec<Preference>, sqlx::Error> {
    match scope {
        Some(s) => {
            sqlx::query_as::<_, Preference>(
                "SELECT * FROM preferences WHERE scope = $1 ORDER BY key",
            )
            .bind(s)
            .fetch_all(pool)
            .await
        }
        None => {
            sqlx::query_as::<_, Preference>("SELECT * FROM preferences ORDER BY scope, key")
                .fetch_all(pool)
                .await
        }
    }
}

/// Delete a preference by scope and key. Returns true if a row was deleted.
pub async fn delete_preference(pool: &PgPool, scope: &str, key: &str) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM preferences WHERE scope = $1 AND key = $2")
        .bind(scope)
        .bind(key)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
