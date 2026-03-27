use serde::Serialize;
use sqlx::PgPool;

use crate::models::client::Client;
use crate::models::handoff::Handoff;
use crate::models::incident::Incident;
use crate::models::knowledge::Knowledge;
use crate::models::network::Network;
use crate::models::runbook::Runbook;
use crate::models::server::Server;
use crate::models::service::Service;
use crate::models::site::Site;
use crate::models::vendor::Vendor;

#[derive(Debug, Clone, Serialize)]
pub struct SearchResults {
    pub servers: Vec<Server>,
    pub services: Vec<Service>,
    pub runbooks: Vec<Runbook>,
    pub knowledge: Vec<Knowledge>,
    pub incidents: Vec<Incident>,
    pub handoffs: Vec<Handoff>,
    pub vendors: Vec<Vendor>,
    pub clients: Vec<Client>,
    pub sites: Vec<Site>,
    pub networks: Vec<Network>,
}

pub async fn search_inventory(
    pool: &PgPool,
    query: &str,
    limit_per_type: i64,
) -> Result<SearchResults, sqlx::Error> {
    let (servers, services, runbooks, knowledge, incidents, handoffs) = tokio::try_join!(
        sqlx::query_as::<_, Server>(
            "SELECT * FROM servers
             WHERE status != 'deleted' AND search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
        sqlx::query_as::<_, Service>(
            "SELECT * FROM services
             WHERE status != 'deleted' AND search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
        sqlx::query_as::<_, Runbook>(
            "SELECT * FROM runbooks
             WHERE search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
        sqlx::query_as::<_, Knowledge>(
            "SELECT * FROM knowledge
             WHERE search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
        sqlx::query_as::<_, Incident>(
            "SELECT * FROM incidents
             WHERE search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
        sqlx::query_as::<_, Handoff>(
            "SELECT * FROM handoffs
             WHERE search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
    )?;

    let (vendors, clients, sites, networks) = tokio::try_join!(
        sqlx::query_as::<_, Vendor>(
            "SELECT * FROM vendors
             WHERE status != 'deleted' AND search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
        sqlx::query_as::<_, Client>(
            "SELECT * FROM clients
             WHERE search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
        sqlx::query_as::<_, Site>(
            "SELECT * FROM sites
             WHERE search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
        sqlx::query_as::<_, Network>(
            "SELECT * FROM networks
             WHERE search_vector @@ websearch_to_tsquery('english', $1)
             ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
             LIMIT $2"
        )
        .bind(query)
        .bind(limit_per_type)
        .fetch_all(pool),
    )?;

    Ok(SearchResults {
        servers,
        services,
        runbooks,
        knowledge,
        incidents,
        handoffs,
        vendors,
        clients,
        sites,
        networks,
    })
}

pub async fn search_runbooks(
    pool: &PgPool,
    query: &str,
    limit: i64,
) -> Result<Vec<Runbook>, sqlx::Error> {
    let results = sqlx::query_as::<_, Runbook>(
        "SELECT * FROM runbooks
         WHERE search_vector @@ websearch_to_tsquery('english', $1)
         ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
         LIMIT $2",
    )
    .bind(query)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    if results.is_empty() {
        if let Some(or_text) = super::build_or_tsquery_text(query) {
            return sqlx::query_as::<_, Runbook>(
                "SELECT * FROM runbooks
                 WHERE search_vector @@ to_tsquery('english', $1)
                 ORDER BY ts_rank(search_vector, to_tsquery('english', $1)) DESC
                 LIMIT $2",
            )
            .bind(&or_text)
            .bind(limit)
            .fetch_all(pool)
            .await;
        }
    }

    Ok(results)
}
