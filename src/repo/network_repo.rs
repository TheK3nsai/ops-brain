use sqlx::PgPool;
use uuid::Uuid;

use crate::models::network::Network;

pub async fn get_network(pool: &PgPool, id: Uuid) -> Result<Option<Network>, sqlx::Error> {
    sqlx::query_as::<_, Network>("SELECT * FROM networks WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list_networks(
    pool: &PgPool,
    site_id: Option<Uuid>,
) -> Result<Vec<Network>, sqlx::Error> {
    match site_id {
        Some(sid) => {
            sqlx::query_as::<_, Network>("SELECT * FROM networks WHERE site_id = $1 ORDER BY name")
                .bind(sid)
                .fetch_all(pool)
                .await
        }
        None => {
            sqlx::query_as::<_, Network>("SELECT * FROM networks ORDER BY name")
                .fetch_all(pool)
                .await
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_network(
    pool: &PgPool,
    site_id: Uuid,
    name: &str,
    cidr: &str,
    vlan_id: Option<i32>,
    gateway: Option<&str>,
    dns_servers: &[String],
    dhcp_server: Option<&str>,
    purpose: Option<&str>,
    notes: Option<&str>,
) -> Result<Network, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Network>(
        "INSERT INTO networks (id, site_id, name, cidr, vlan_id, gateway, dns_servers,
            dhcp_server, purpose, notes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         ON CONFLICT (site_id, cidr) DO UPDATE SET
             name = EXCLUDED.name,
             vlan_id = EXCLUDED.vlan_id,
             gateway = EXCLUDED.gateway,
             dns_servers = EXCLUDED.dns_servers,
             dhcp_server = EXCLUDED.dhcp_server,
             purpose = EXCLUDED.purpose,
             notes = EXCLUDED.notes,
             updated_at = NOW()
         RETURNING *",
    )
    .bind(id)
    .bind(site_id)
    .bind(name)
    .bind(cidr)
    .bind(vlan_id)
    .bind(gateway)
    .bind(dns_servers)
    .bind(dhcp_server)
    .bind(purpose)
    .bind(notes)
    .fetch_one(pool)
    .await
}
