use sqlx::PgPool;
use uuid::Uuid;

use crate::models::ticket_link::TicketLink;

pub async fn create_link(
    pool: &PgPool,
    zammad_ticket_id: i32,
    incident_id: Option<Uuid>,
    server_id: Option<Uuid>,
    service_id: Option<Uuid>,
    notes: Option<&str>,
) -> Result<TicketLink, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, TicketLink>(
        "INSERT INTO ticket_links (id, zammad_ticket_id, incident_id, server_id, service_id, notes)
         VALUES ($1, $2, $3, $4, $5, $6)
         ON CONFLICT (zammad_ticket_id) DO UPDATE SET
             incident_id = COALESCE(EXCLUDED.incident_id, ticket_links.incident_id),
             server_id = COALESCE(EXCLUDED.server_id, ticket_links.server_id),
             service_id = COALESCE(EXCLUDED.service_id, ticket_links.service_id),
             notes = COALESCE(EXCLUDED.notes, ticket_links.notes),
             updated_at = now()
         RETURNING *",
    )
    .bind(id)
    .bind(zammad_ticket_id)
    .bind(incident_id)
    .bind(server_id)
    .bind(service_id)
    .bind(notes)
    .fetch_one(pool)
    .await
}

pub async fn delete_link(pool: &PgPool, zammad_ticket_id: i32) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM ticket_links WHERE zammad_ticket_id = $1")
        .bind(zammad_ticket_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn get_link_by_ticket_id(
    pool: &PgPool,
    zammad_ticket_id: i32,
) -> Result<Option<TicketLink>, sqlx::Error> {
    sqlx::query_as::<_, TicketLink>(
        "SELECT * FROM ticket_links WHERE zammad_ticket_id = $1",
    )
    .bind(zammad_ticket_id)
    .fetch_optional(pool)
    .await
}

pub async fn get_links_for_incident(
    pool: &PgPool,
    incident_id: Uuid,
) -> Result<Vec<TicketLink>, sqlx::Error> {
    sqlx::query_as::<_, TicketLink>(
        "SELECT * FROM ticket_links WHERE incident_id = $1 ORDER BY created_at DESC",
    )
    .bind(incident_id)
    .fetch_all(pool)
    .await
}

pub async fn get_links_for_server(
    pool: &PgPool,
    server_id: Uuid,
) -> Result<Vec<TicketLink>, sqlx::Error> {
    sqlx::query_as::<_, TicketLink>(
        "SELECT * FROM ticket_links WHERE server_id = $1 ORDER BY created_at DESC",
    )
    .bind(server_id)
    .fetch_all(pool)
    .await
}

pub async fn get_links_for_service(
    pool: &PgPool,
    service_id: Uuid,
) -> Result<Vec<TicketLink>, sqlx::Error> {
    sqlx::query_as::<_, TicketLink>(
        "SELECT * FROM ticket_links WHERE service_id = $1 ORDER BY created_at DESC",
    )
    .bind(service_id)
    .fetch_all(pool)
    .await
}
