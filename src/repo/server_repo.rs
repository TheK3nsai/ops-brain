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
    sqlx::query_as::<_, Server>("SELECT * FROM servers WHERE slug = $1 AND status != 'deleted'")
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
    limit: i64,
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
        param_idx += 1;
    } else {
        // Exclude soft-deleted by default
        conditions.push("s.status != 'deleted'".to_string());
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY s.hostname");
    query.push_str(&format!(" LIMIT ${param_idx}"));

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
    q = q.bind(limit);

    q.fetch_all(pool).await
}

#[allow(clippy::too_many_arguments)]
/// Count references to a server across junction tables.
/// Returns a map of table_name -> count for non-zero references.
pub async fn count_server_references(
    pool: &PgPool,
    server_id: Uuid,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let row: (i64, i64, i64, i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM server_services WHERE server_id = $1),
            (SELECT COUNT(*) FROM incident_servers WHERE server_id = $1),
            (SELECT COUNT(*) FROM monitors WHERE server_id = $1),
            (SELECT COUNT(*) FROM ticket_links WHERE server_id = $1),
            (SELECT COUNT(*) FROM servers WHERE hypervisor_id = $1)",
    )
    .bind(server_id)
    .fetch_one(pool)
    .await?;

    let mut refs = Vec::new();
    if row.0 > 0 {
        refs.push(("linked services".to_string(), row.0));
    }
    if row.1 > 0 {
        refs.push(("incident links".to_string(), row.1));
    }
    if row.2 > 0 {
        refs.push(("monitor mappings".to_string(), row.2));
    }
    if row.3 > 0 {
        refs.push(("ticket links".to_string(), row.3));
    }
    if row.4 > 0 {
        refs.push(("child VMs (hypervisor_id)".to_string(), row.4));
    }
    Ok(refs)
}

pub async fn delete_server(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result =
        sqlx::query("UPDATE servers SET status = 'deleted', updated_at = NOW() WHERE id = $1")
            .bind(id)
            .execute(pool)
            .await?;
    Ok(result.rows_affected() > 0)
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

/// Partial update: only updates fields that are explicitly provided (Some).
/// NOT NULL columns (ip_addresses, roles, is_virtual, status) are preserved
/// when the caller passes None — unlike upsert_server which replaces all fields.
#[allow(clippy::too_many_arguments)]
pub async fn update_server_partial(
    pool: &PgPool,
    slug: &str,
    site_id: Option<Uuid>,
    hostname: Option<&str>,
    os: Option<&str>,
    ip_addresses: Option<&[String]>,
    ssh_alias: Option<&str>,
    roles: Option<&[String]>,
    hardware: Option<&str>,
    cpu: Option<&str>,
    ram_gb: Option<i32>,
    storage_summary: Option<&str>,
    is_virtual: Option<bool>,
    hypervisor_id: Option<Option<Uuid>>,
    status: Option<&str>,
    notes: Option<&str>,
) -> Result<Server, sqlx::Error> {
    sqlx::query_as::<_, Server>(
        "UPDATE servers SET
            site_id = COALESCE($2, site_id),
            hostname = COALESCE($3, hostname),
            os = COALESCE($4, os),
            ip_addresses = COALESCE($5, ip_addresses),
            ssh_alias = COALESCE($6, ssh_alias),
            roles = COALESCE($7, roles),
            hardware = COALESCE($8, hardware),
            cpu = COALESCE($9, cpu),
            ram_gb = COALESCE($10, ram_gb),
            storage_summary = COALESCE($11, storage_summary),
            is_virtual = COALESCE($12, is_virtual),
            hypervisor_id = CASE WHEN $13 THEN $14 ELSE hypervisor_id END,
            status = COALESCE($15, status),
            notes = COALESCE($16, notes),
            updated_at = NOW()
         WHERE slug = $1 AND status != 'deleted'
         RETURNING *",
    )
    .bind(slug)
    .bind(site_id)
    .bind(hostname)
    .bind(os)
    .bind(ip_addresses.map(|a| a.to_vec()))
    .bind(ssh_alias)
    .bind(roles.map(|r| r.to_vec()))
    .bind(hardware)
    .bind(cpu)
    .bind(ram_gb)
    .bind(storage_summary)
    .bind(is_virtual)
    // $13 = whether hypervisor_id was explicitly provided, $14 = the value (may be NULL)
    .bind(hypervisor_id.is_some())
    .bind(hypervisor_id.flatten())
    .bind(status)
    .bind(notes)
    .fetch_one(pool)
    .await
}
