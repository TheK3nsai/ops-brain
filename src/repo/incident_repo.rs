use sqlx::PgPool;
use uuid::Uuid;

use crate::models::incident::Incident;

pub async fn get_incident(pool: &PgPool, id: Uuid) -> Result<Option<Incident>, sqlx::Error> {
    sqlx::query_as::<_, Incident>("SELECT * FROM incidents WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn list_incidents(
    pool: &PgPool,
    client_id: Option<Uuid>,
    status: Option<&str>,
    severity: Option<&str>,
    limit: i64,
) -> Result<Vec<Incident>, sqlx::Error> {
    let mut query = String::from("SELECT * FROM incidents");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if client_id.is_some() {
        conditions.push(format!("client_id = ${param_idx}"));
        param_idx += 1;
    }
    if status.is_some() {
        conditions.push(format!("status = ${param_idx}"));
        param_idx += 1;
    }
    if severity.is_some() {
        conditions.push(format!("severity = ${param_idx}"));
        param_idx += 1;
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY reported_at DESC");
    query.push_str(&format!(" LIMIT ${param_idx}"));

    let mut q = sqlx::query_as::<_, Incident>(&query);
    if let Some(v) = client_id {
        q = q.bind(v);
    }
    if let Some(v) = status {
        q = q.bind(v);
    }
    if let Some(v) = severity {
        q = q.bind(v);
    }
    q = q.bind(limit);

    q.fetch_all(pool).await
}

#[allow(clippy::too_many_arguments)]
pub async fn create_incident(
    pool: &PgPool,
    title: &str,
    severity: &str,
    client_id: Option<Uuid>,
    symptoms: Option<&str>,
    notes: Option<&str>,
) -> Result<Incident, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Incident>(
        "INSERT INTO incidents (id, title, status, severity, client_id, symptoms, notes)
         VALUES ($1, $2, 'open', $3, $4, $5, $6)
         RETURNING *",
    )
    .bind(id)
    .bind(title)
    .bind(severity)
    .bind(client_id)
    .bind(symptoms)
    .bind(notes)
    .fetch_one(pool)
    .await
}

#[allow(clippy::too_many_arguments)]
pub async fn update_incident(
    pool: &PgPool,
    id: Uuid,
    title: Option<&str>,
    status: Option<&str>,
    severity: Option<&str>,
    symptoms: Option<&str>,
    root_cause: Option<&str>,
    resolution: Option<&str>,
    prevention: Option<&str>,
    notes: Option<&str>,
) -> Result<Incident, sqlx::Error> {
    // If resolving, calculate TTR and set resolved_at
    let incident = sqlx::query_as::<_, Incident>(
        "UPDATE incidents SET
            title = COALESCE($2, title),
            status = COALESCE($3, status),
            severity = COALESCE($4, severity),
            symptoms = COALESCE($5, symptoms),
            root_cause = COALESCE($6, root_cause),
            resolution = COALESCE($7, resolution),
            prevention = COALESCE($8, prevention),
            notes = COALESCE($9, notes),
            resolved_at = CASE
                WHEN $3 = 'resolved' AND resolved_at IS NULL THEN now()
                ELSE resolved_at
            END,
            time_to_resolve_minutes = CASE
                WHEN $3 = 'resolved' AND resolved_at IS NULL
                    THEN EXTRACT(EPOCH FROM (now() - reported_at))::integer / 60
                ELSE time_to_resolve_minutes
            END,
            updated_at = NOW()
         WHERE id = $1 RETURNING *",
    )
    .bind(id)
    .bind(title)
    .bind(status)
    .bind(severity)
    .bind(symptoms)
    .bind(root_cause)
    .bind(resolution)
    .bind(prevention)
    .bind(notes)
    .fetch_one(pool)
    .await?;

    Ok(incident)
}

pub async fn search_incidents(
    pool: &PgPool,
    query: &str,
) -> Result<Vec<Incident>, sqlx::Error> {
    sqlx::query_as::<_, Incident>(
        "SELECT * FROM incidents
         WHERE search_vector @@ plainto_tsquery('english', $1)
         ORDER BY ts_rank(search_vector, plainto_tsquery('english', $1)) DESC
         LIMIT 20",
    )
    .bind(query)
    .fetch_all(pool)
    .await
}

pub async fn link_incident_server(
    pool: &PgPool,
    incident_id: Uuid,
    server_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO incident_servers (incident_id, server_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(incident_id)
    .bind(server_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn link_incident_service(
    pool: &PgPool,
    incident_id: Uuid,
    service_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO incident_services (incident_id, service_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(incident_id)
    .bind(service_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn link_incident_runbook(
    pool: &PgPool,
    incident_id: Uuid,
    runbook_id: Uuid,
    usage: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO incident_runbooks (incident_id, runbook_id, usage)
         VALUES ($1, $2, $3)
         ON CONFLICT (incident_id, runbook_id) DO UPDATE SET usage = $3",
    )
    .bind(incident_id)
    .bind(runbook_id)
    .bind(usage)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn link_incident_vendor(
    pool: &PgPool,
    incident_id: Uuid,
    vendor_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO incident_vendors (incident_id, vendor_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(incident_id)
    .bind(vendor_id)
    .execute(pool)
    .await?;
    Ok(())
}

/// Get incidents linked to a specific server
pub async fn get_incidents_for_server(
    pool: &PgPool,
    server_id: Uuid,
    limit: i64,
) -> Result<Vec<Incident>, sqlx::Error> {
    sqlx::query_as::<_, Incident>(
        "SELECT i.* FROM incidents i
         JOIN incident_servers isv ON i.id = isv.incident_id
         WHERE isv.server_id = $1
         ORDER BY i.reported_at DESC
         LIMIT $2",
    )
    .bind(server_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Get incidents for a client
pub async fn get_incidents_for_client(
    pool: &PgPool,
    client_id: Uuid,
    limit: i64,
) -> Result<Vec<Incident>, sqlx::Error> {
    sqlx::query_as::<_, Incident>(
        "SELECT * FROM incidents WHERE client_id = $1
         ORDER BY reported_at DESC LIMIT $2",
    )
    .bind(client_id)
    .bind(limit)
    .fetch_all(pool)
    .await
}
