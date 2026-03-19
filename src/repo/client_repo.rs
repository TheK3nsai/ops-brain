use sqlx::PgPool;
use uuid::Uuid;

use crate::models::client::Client;

pub async fn get_client(pool: &PgPool, id: Uuid) -> Result<Option<Client>, sqlx::Error> {
    sqlx::query_as::<_, Client>("SELECT * FROM clients WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_client_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Client>, sqlx::Error> {
    sqlx::query_as::<_, Client>("SELECT * FROM clients WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
}

pub async fn list_clients(pool: &PgPool) -> Result<Vec<Client>, sqlx::Error> {
    sqlx::query_as::<_, Client>("SELECT * FROM clients ORDER BY name")
        .fetch_all(pool)
        .await
}

pub async fn upsert_client(
    pool: &PgPool,
    name: &str,
    slug: &str,
    notes: Option<&str>,
) -> Result<Client, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Client>(
        "INSERT INTO clients (id, name, slug, notes)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (slug) DO UPDATE SET
             name = EXCLUDED.name,
             notes = EXCLUDED.notes,
             updated_at = NOW()
         RETURNING *",
    )
    .bind(id)
    .bind(name)
    .bind(slug)
    .bind(notes)
    .fetch_one(pool)
    .await
}
