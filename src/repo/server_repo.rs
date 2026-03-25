use sqlx::PgPool;
use uuid::Uuid;

use crate::models::server::Server;

pub async fn get_server(pool: &PgPool, id: Uuid) -> Result<Option<Server>, sqlx::Error> {
    sqlx::query_as::<_, Server>("SELECT * FROM servers WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_server_by_slug(pool: &PgPool, slug: &str) -> Result<Option<Server>, sqlx::Error> {
    sqlx::query_as::<_, Server>("SELECT * FROM servers WHERE slug = $1")
        .bind(slug)
        .fetch_optional(pool)
        .await
}

pub async fn list_servers(
    pool: &PgPool,
    client_id: Option<Uuid>,
    site_id: Option<Uuid>,
    role: Option<&str>,
    status: Option<&str>,
) -> Result<Vec<Server>, sqlx::Error> {
    let mut query = String::from("SELECT s.* FROM servers s");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if client_id.is_some() {
        query.push_str(" JOIN sites st ON s.site_id = st.id");
        conditions.push(format!("st.client_id = ${param_idx}"));
        param_idx += 1;
    }
    if site_id.is_some() {
        conditions.push(format!("s.site_id = ${param_idx}"));
        param_idx += 1;
    }
    if role.is_some() {
        conditions.push(format!("${param_idx} = ANY(s.roles)"));
        param_idx += 1;
    }
    if status.is_some() {
        conditions.push(format!("s.status = ${param_idx}"));
        let _ = param_idx; // last one, no need to increment
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY s.hostname");

    let mut q = sqlx::query_as::<_, Server>(&query);
    if let Some(v) = client_id {
        q = q.bind(v);
    }
    if let Some(v) = site_id {
        q = q.bind(v);
    }
    if let Some(v) = role {
        q = q.bind(v);
    }
    if let Some(v) = status {
        q = q.bind(v);
    }

    q.fetch_all(pool).await
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_server(
    pool: &PgPool,
    site_id: Uuid,
    hostname: &str,
    slug: &str,
    os: Option<&str>,
    ip_addresses: &[String],
    ssh_alias: Option<&str>,
    roles: &[String],
    hardware: Option<&str>,
    cpu: Option<&str>,
    ram_gb: Option<i32>,
    storage_summary: Option<&str>,
    is_virtual: bool,
    hypervisor_id: Option<Uuid>,
    status: &str,
    notes: Option<&str>,
) -> Result<Server, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Server>(
        "INSERT INTO servers (id, site_id, hostname, slug, os, ip_addresses, ssh_alias, roles,
            hardware, cpu, ram_gb, storage_summary, is_virtual, hypervisor_id, status, notes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12, $13, $14, $15, $16)
         ON CONFLICT (slug) DO UPDATE SET
             site_id = EXCLUDED.site_id,
             hostname = EXCLUDED.hostname,
             os = EXCLUDED.os,
             ip_addresses = EXCLUDED.ip_addresses,
             ssh_alias = EXCLUDED.ssh_alias,
             roles = EXCLUDED.roles,
             hardware = EXCLUDED.hardware,
             cpu = EXCLUDED.cpu,
             ram_gb = EXCLUDED.ram_gb,
             storage_summary = EXCLUDED.storage_summary,
             is_virtual = EXCLUDED.is_virtual,
             hypervisor_id = EXCLUDED.hypervisor_id,
             status = EXCLUDED.status,
             notes = EXCLUDED.notes,
             updated_at = NOW()
         RETURNING *",
    )
    .bind(id)
    .bind(site_id)
    .bind(hostname)
    .bind(slug)
    .bind(os)
    .bind(ip_addresses)
    .bind(ssh_alias)
    .bind(roles)
    .bind(hardware)
    .bind(cpu)
    .bind(ram_gb)
    .bind(storage_summary)
    .bind(is_virtual)
    .bind(hypervisor_id)
    .bind(status)
    .bind(notes)
    .fetch_one(pool)
    .await
}
