use sqlx::PgPool;
use uuid::Uuid;

use crate::models::server::Server;
use crate::models::service::{Service, ServiceWithPort};

pub async fn get_service(pool: &PgPool, id: Uuid) -> Result<Option<Service>, sqlx::Error> {
    sqlx::query_as::<_, Service>("SELECT * FROM services WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_service_by_slug(
    pool: &PgPool,
    slug: &str,
) -> Result<Option<Service>, sqlx::Error> {
    sqlx::query_as::<_, Service>("SELECT * FROM services WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
}

pub async fn list_services(
    pool: &PgPool,
    category: Option<&str>,
) -> Result<Vec<Service>, sqlx::Error> {
    match category {
        Some(cat) => {
            sqlx::query_as::<_, Service>("SELECT * FROM services WHERE category = $1 ORDER BY name")
                .bind(cat)
                .fetch_all(pool)
                .await
        }
        None => {
            sqlx::query_as::<_, Service>("SELECT * FROM services ORDER BY name")
                .fetch_all(pool)
                .await
        }
    }
}

pub async fn upsert_service(
    pool: &PgPool,
    name: &str,
    slug: &str,
    category: Option<&str>,
    description: Option<&str>,
    criticality: &str,
    notes: Option<&str>,
) -> Result<Service, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Service>(
        "INSERT INTO services (id, name, slug, category, description, criticality, notes)
         VALUES ($1, $2, $3, $4, $5, $6, $7)
         ON CONFLICT (slug) DO UPDATE SET
             name = EXCLUDED.name,
             category = EXCLUDED.category,
             description = EXCLUDED.description,
             criticality = EXCLUDED.criticality,
             notes = EXCLUDED.notes,
             updated_at = NOW()
         RETURNING *",
    )
    .bind(id)
    .bind(name)
    .bind(slug)
    .bind(category)
    .bind(description)
    .bind(criticality)
    .bind(notes)
    .fetch_one(pool)
    .await
}

pub async fn link_server_service(
    pool: &PgPool,
    server_id: Uuid,
    service_id: Uuid,
    port: Option<i32>,
    config_notes: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO server_services (server_id, service_id, port, config_notes)
         VALUES ($1, $2, $3, $4)
         ON CONFLICT (server_id, service_id) DO UPDATE SET
             port = EXCLUDED.port,
             config_notes = EXCLUDED.config_notes",
    )
    .bind(server_id)
    .bind(service_id)
    .bind(port)
    .bind(config_notes)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_services_for_server(
    pool: &PgPool,
    server_id: Uuid,
) -> Result<Vec<ServiceWithPort>, sqlx::Error> {
    sqlx::query_as::<_, ServiceWithPort>(
        "SELECT sv.id, sv.name, sv.slug, sv.category, sv.description, sv.criticality, sv.notes,
                ss.port, ss.config_notes
         FROM services sv
         JOIN server_services ss ON sv.id = ss.service_id
         WHERE ss.server_id = $1
         ORDER BY sv.name",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn get_servers_for_service(
    pool: &PgPool,
    service_id: Uuid,
) -> Result<Vec<Server>, sqlx::Error> {
    sqlx::query_as::<_, Server>(
        "SELECT s.*
         FROM servers s
         JOIN server_services ss ON s.id = ss.server_id
         WHERE ss.service_id = $1
         ORDER BY s.hostname",
    )
    .bind(service_id)
    .fetch_all(pool)
    .await
}
