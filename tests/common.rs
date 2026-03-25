//! Shared test utilities for integration tests.

use sqlx::PgPool;

/// Create a test database pool.
///
/// Uses DATABASE_URL from environment (defaults to local dev DB).
/// Each test should use transactions that get rolled back, or use unique
/// data to avoid conflicts.
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
