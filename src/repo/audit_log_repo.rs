use sqlx::PgPool;
use uuid::Uuid;

/// Log a cross-client data access attempt to the audit trail.
/// action: "withheld" or "released"
pub async fn log_access(
    pool: &PgPool,
    tool_name: &str,
    requesting_client_id: Option<Uuid>,
    entity_type: &str,
    entity_id: Uuid,
    owning_client_id: Option<Uuid>,
    action: &str,
) {
    let id = Uuid::now_v7();
    if let Err(e) = sqlx::query(
        "INSERT INTO audit_log (id, tool_name, requesting_client_id, entity_type, entity_id, owning_client_id, action)
         VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind(id)
    .bind(tool_name)
    .bind(requesting_client_id)
    .bind(entity_type)
    .bind(entity_id)
    .bind(owning_client_id)
    .bind(action)
    .execute(pool)
    .await
    {
        tracing::warn!("Failed to write audit log: {e}");
    }
}
