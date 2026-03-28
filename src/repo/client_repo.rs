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
    zammad_org_id: Option<i32>,
    zammad_group_id: Option<i32>,
    zammad_customer_id: Option<i32>,
) -> Result<Client, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Client>(
        "INSERT INTO clients (id, name, slug, notes, zammad_org_id, zammad_group_id, zammad_customer_id)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (slug) DO UPDATE SET
             name = EXCLUDED.name,
             notes = COALESCE(EXCLUDED.notes, clients.notes),
             zammad_org_id = COALESCE(EXCLUDED.zammad_org_id, clients.zammad_org_id),
             zammad_group_id = COALESCE(EXCLUDED.zammad_group_id, clients.zammad_group_id),
             zammad_customer_id = COALESCE(EXCLUDED.zammad_customer_id, clients.zammad_customer_id),
             updated_at = NOW()
         RETURNING *",
    )
    .bind(id)
    .bind(name)
    .bind(slug)
    .bind(notes)
    .bind(zammad_org_id)
    .bind(zammad_group_id)
    .bind(zammad_customer_id)
    .fetch_one(pool)
    .await
}
