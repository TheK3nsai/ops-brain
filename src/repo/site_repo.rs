use sqlx::PgPool;
use uuid::Uuid;

use crate::models::site::Site;

pub async fn get_site(pool: &PgPool, id: Uuid) -> Result<Option<Site>, sqlx::Error> {
    sqlx::query_as::<_, Site>("SELECT * FROM sites WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_site_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Site>, sqlx::Error> {
    sqlx::query_as::<_, Site>("SELECT * FROM sites WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
}

pub async fn list_sites(pool: &PgPool, client_id: Option<Uuid>) -> Result<Vec<Site>, sqlx::Error> {
    match client_id {
        Some(cid) => {
            sqlx::query_as::<_, Site>("SELECT * FROM sites WHERE client_id = $1 ORDER BY name")
                .bind(cid)
                .fetch_all(pool)
                .await
        }
        None => {
            sqlx::query_as::<_, Site>("SELECT * FROM sites ORDER BY name")
                .fetch_all(pool)
                .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_site(
    pool: &PgPool,
    client_id: Uuid,
    name: &str,
    slug: &str,
    address: Option<&str>,
    wan_provider: Option<&str>,
    wan_ip: Option<&str>,
    notes: Option<&str>,
) -> Result<Site, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Site>(
        "INSERT INTO sites (id, client_id, name, slug, address, wan_provider, wan_ip, notes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
         ON CONFLICT (slug) DO UPDATE SET
             client_id = EXCLUDED.client_id,
             name = EXCLUDED.name,
             address = EXCLUDED.address,
             wan_provider = EXCLUDED.wan_provider,
             wan_ip = EXCLUDED.wan_ip,
             notes = EXCLUDED.notes,
             updated_at = NOW()
         RETURNING *",
    )
    .bind(id)
    .bind(client_id)
    .bind(name)
    .bind(slug)
    .bind(address)
    .bind(wan_provider)
    .bind(wan_ip)
    .bind(notes)
    .fetch_one(pool)
    .await
}
