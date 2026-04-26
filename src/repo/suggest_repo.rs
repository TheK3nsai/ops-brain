use sqlx::PgPool;

/// Suggest similar slugs from a table using pg_trgm trigram similarity.
///
/// Tries similarity match first (threshold 0.2), then falls back to ILIKE substring.
/// Returns up to 3 suggestions ordered by similarity score.
pub async fn suggest_similar_slugs(pool: &PgPool, table: &str, attempted: &str) -> Vec<String> {
    // Whitelist table names to prevent SQL injection
    let column = "slug";
    let table = match table {
        "servers" | "services" | "sites" | "clients" => table,
        _ => return Vec::new(),
    };

    let query = format!(
        "SELECT {column} FROM {table} \
         WHERE similarity({column}, $1) > 0.2 \
            OR {column} ILIKE '%' || $1 || '%' \
         ORDER BY similarity({column}, $1) DESC \
         LIMIT 3"
    );

    sqlx::query_scalar::<_, String>(&query)
        .bind(attempted)
        .fetch_all(pool)
        .await
        .unwrap_or_default()
}

/// Suggest similar vendor names using pg_trgm trigram similarity.
pub async fn suggest_similar_vendor_names(pool: &PgPool, attempted: &str) -> Vec<String> {
    sqlx::query_scalar::<_, String>(
        "SELECT name FROM vendors \
         WHERE similarity(name, $1) > 0.2 \
            OR name ILIKE '%' || $1 || '%' \
         ORDER BY similarity(name, $1) DESC \
         LIMIT 3",
    )
    .bind(attempted)
    .fetch_all(pool)
    .await
    .unwrap_or_default()
}
