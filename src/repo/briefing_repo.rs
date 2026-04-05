use sqlx::PgPool;
use uuid::Uuid;

use crate::models::briefing::Briefing;

pub async fn insert_briefing(
    pool: &PgPool,
    briefing_type: &str,
    client_id: Option<Uuid>,
    content: &str,
) -> Result<Briefing, sqlx::Error> {
    let id = Uuid::now_v7();
    sqlx::query_as::<_, Briefing>(
        "INSERT INTO briefings (id, briefing_type, client_id, content)
         VALUES ($1, $2, $3, $4)
         RETURNING *",
    )
    .bind(id)
    .bind(briefing_type)
    .bind(client_id)
    .bind(content)
    .fetch_one(pool)
    .await
}
