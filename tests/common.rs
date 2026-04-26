//! Shared test utilities for integration tests.
//!
//! Requires a running PostgreSQL instance. Uses DATABASE_URL from environment
//! or defaults to the test database.
//!
//! Isolation model: each test uses UUID-based unique slugs/names to avoid
//! conflicts. Tests clean up their own data on success. If a test panics,
//! orphaned rows are harmless (unique IDs) but accumulate. Run
//! `cleanup_stale_test_data()` periodically or at the start of test suites
//! to purge old test artifacts.
//!
//! Run: DATABASE_URL=postgres://ops_brain:ops_brain@localhost:5432/ops_brain cargo test

use sqlx::PgPool;

/// Create a test database pool.
///
/// Uses DATABASE_URL from environment (defaults to local dev DB).
pub async fn test_pool() -> PgPool {
    dotenvy::dotenv().ok();

    let url = std::env::var("DATABASE_URL")
        .unwrap_or_else(|_| "postgres://ops_brain:ops_brain@localhost:5432/ops_brain_test".into());

    let pool = sqlx::postgres::PgPoolOptions::new()
        .max_connections(5)
        .connect(&url)
        .await
        .expect("Failed to connect to test database");

    // Run migrations
    sqlx::migrate!("./migrations")
        .run(&pool)
        .await
        .expect("Failed to run migrations");

    pool
}

/// Remove test artifacts older than 1 hour.
/// Safe to call at any time — only deletes rows with UUID-based test slugs.
#[allow(dead_code)]
pub async fn cleanup_stale_test_data(pool: &PgPool) {
    // Test slugs contain UUIDs, so they're much longer than real slugs.
    // Clean up anything with a test-pattern slug older than 1 hour.
    let tables_with_slugs = ["clients", "sites", "servers", "services"];
    for table in tables_with_slugs {
        let query = format!(
            "DELETE FROM {table} WHERE slug LIKE 'test-%' AND created_at < NOW() - INTERVAL '1 hour'"
        );
        let _ = sqlx::query(&query).execute(pool).await;
    }
}
