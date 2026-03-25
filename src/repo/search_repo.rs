use serde::Serialize;
use sqlx::PgPool;

use crate::models::handoff::Handoff;
use crate::models::incident::Incident;
use crate::models::knowledge::Knowledge;
use crate::models::runbook::Runbook;
use crate::models::server::Server;
use crate::models::service::Service;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub servers: Vec<Server>,
    pub services: Vec<Service>,
    pub runbooks: Vec<Runbook>,
    pub knowledge: Vec<Knowledge>,
    pub incidents: Vec<Incident>,
    pub handoffs: Vec<Handoff>,
}

pub async fn search_inventory(pool: &PgPool, query: &str) -> Result<SearchResults, sqlx::Error> {
    let (servers, services, runbooks, knowledge, incidents, handoffs) = tokio::try_join!(
        sqlx::query_as::<_, Server>(
            "SELECT * FROM servers
             WHERE search_vector @@ plainto_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
             LIMIT 10"
        )
        .bind(query)
        .fetch_all(pool),
        sqlx::query_as::<_, Service>(
            "SELECT * FROM services
             WHERE search_vector @@ plainto_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
             LIMIT 10"
        )
        .bind(query)
        .fetch_all(pool),
        sqlx::query_as::<_, Runbook>(
            "SELECT * FROM runbooks
             WHERE search_vector @@ plainto_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
             LIMIT 10"
        )
        .bind(query)
        .fetch_all(pool),
        sqlx::query_as::<_, Knowledge>(
            "SELECT * FROM knowledge
             WHERE search_vector @@ plainto_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
             LIMIT 10"
        )
        .bind(query)
        .fetch_all(pool),
        sqlx::query_as::<_, Incident>(
            "SELECT * FROM incidents
             WHERE search_vector @@ plainto_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
             LIMIT 10"
        )
        .bind(query)
        .fetch_all(pool),
        sqlx::query_as::<_, Handoff>(
            "SELECT * FROM handoffs
             WHERE search_vector @@ plainto_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
             LIMIT 10"
        )
        .bind(query)
        .fetch_all(pool),
    )?;

    Ok(SearchResults {
        servers,
        services,
        runbooks,
        knowledge,
        incidents,
        handoffs,
    })
}

pub async fn search_runbooks(pool: &PgPool, query: &str) -> Result<Vec<Runbook>, sqlx::Error> {
    sqlx::query_as::<_, Runbook>(
        "SELECT * FROM runbooks
         WHERE search_vector @@ plainto_tsquery('english', $1)
         ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC",
    )
    .bind(query)
    .fetch_all(pool)
    .await
}
