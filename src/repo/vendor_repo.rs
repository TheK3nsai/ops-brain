use chrono::NaiveDate;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::vendor::Vendor;

pub async fn get_vendor(pool: &PgPool, id: Uuid) -> Result<Option<Vendor>, sqlx::Error> {
    sqlx::query_as::<_, Vendor>("SELECT * FROM vendors WHERE id = $1")
        .bind(id)
        .fetch_optional(pool)
        .await
}

pub async fn get_vendor_by_name(pool: &PgPool, name: &str) -> Result<Option<Vendor>, sqlx::Error> {
    sqlx::query_as::<_, Vendor>("SELECT * FROM vendors WHERE LOWER(name) = LOWER($1)")
        .bind(name)
        .fetch_optional(pool)
        .await
}

pub async fn list_vendors(
    pool: &PgPool,
    client_id: Option<Uuid>,
    category: Option<&str>,
) -> Result<Vec<Vendor>, sqlx::Error> {
    let mut query = String::from("SELECT v.* FROM vendors v");
    let mut conditions: Vec<String> = Vec::new();
    let mut param_idx = 1u32;

    if client_id.is_some() {
        query.push_str(" JOIN vendor_clients vc ON v.id = vc.vendor_id");
        conditions.push(format!("vc.client_id = ${param_idx}"));
        param_idx += 1;
    }
    if category.is_some() {
        conditions.push(format!("v.category = ${param_idx}"));
        let _ = param_idx;
    }

    if !conditions.is_empty() {
        query.push_str(" WHERE ");
        query.push_str(&conditions.join(" AND "));
    }
    query.push_str(" ORDER BY v.name");

    let mut q = sqlx::query_as::<_, Vendor>(&query);
    if let Some(v) = client_id {
        q = q.bind(v);
    }
    if let Some(v) = category {
        q = q.bind(v);
    }

    q.fetch_all(pool).await
}

#[allow(clippy::too_many_arguments)]
/// Count references to a vendor across junction tables.
pub async fn count_vendor_references(
    pool: &PgPool,
    vendor_id: Uuid,
) -> Result<Vec<(String, i64)>, sqlx::Error> {
    let row: (i64, i64) = sqlx::query_as(
        "SELECT
            (SELECT COUNT(*) FROM vendor_clients WHERE vendor_id = $1),
            (SELECT COUNT(*) FROM incident_vendors WHERE vendor_id = $1)",
    )
    .bind(vendor_id)
    .fetch_one(pool)
    .await?;

    let mut refs = Vec::new();
    if row.0 > 0 {
        refs.push(("client links".to_string(), row.0));
    }
    if row.1 > 0 {
        refs.push(("incident links".to_string(), row.1));
    }
    Ok(refs)
}

pub async fn delete_vendor(pool: &PgPool, id: Uuid) -> Result<bool, sqlx::Error> {
    let result = sqlx::query("DELETE FROM vendors WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}

#[allow(clippy::too_many_arguments)]
pub async fn upsert_vendor(
    pool: &PgPool,
    name: &str,
    category: Option<&str>,
    account_number: Option<&str>,
    support_phone: Option<&str>,
    support_email: Option<&str>,
    support_portal: Option<&str>,
    sla_summary: Option<&str>,
    contract_end: Option<NaiveDate>,
    notes: Option<&str>,
) -> Result<Vendor, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Vendor>(
        "INSERT INTO vendors (id, name, category, account_number, support_phone, support_email,
            support_portal, sla_summary, contract_end, notes)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10)
         RETURNING *",
    )
    .bind(id)
    .bind(name)
    .bind(category)
    .bind(account_number)
    .bind(support_phone)
    .bind(support_email)
    .bind(support_portal)
    .bind(sla_summary)
    .bind(contract_end)
    .bind(notes)
    .fetch_one(pool)
    .await
}

pub async fn link_vendor_client(
    pool: &PgPool,
    vendor_id: Uuid,
    client_id: Uuid,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO vendor_clients (vendor_id, client_id)
         VALUES ($1, $2)
         ON CONFLICT DO NOTHING",
    )
    .bind(vendor_id)
    .bind(client_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn get_vendors_for_client(
    pool: &PgPool,
    client_id: Uuid,
) -> Result<Vec<Vendor>, sqlx::Error> {
    sqlx::query_as::<_, Vendor>(
        "SELECT v.*
         FROM vendors v
         JOIN vendor_clients vc ON v.id = vc.vendor_id
         WHERE vc.client_id = $1
         ORDER BY v.name",
    )
    .bind(client_id)
    .fetch_all(pool)
    .await
}
