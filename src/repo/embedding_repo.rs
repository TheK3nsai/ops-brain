use chrono::{DateTime, Utc};
use pgvector::Vector;
use sqlx::PgPool;
use uuid::Uuid;

use crate::models::handoff::Handoff;
use crate::models::incident::Incident;
use crate::models::knowledge::Knowledge;

// ===== STORE EMBEDDINGS =====

pub async fn store_knowledge_embedding(
    pool: &PgPool,
    id: Uuid,
    embedding: &[f32],
) -> Result<(), sqlx::Error> {
    let vec = Vector::from(embedding.to_vec());
    sqlx::query("UPDATE knowledge SET embedding = $1 WHERE id = $2")
        .bind(vec)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn store_incident_embedding(
    pool: &PgPool,
    id: Uuid,
    embedding: &[f32],
) -> Result<(), sqlx::Error> {
    let vec = Vector::from(embedding.to_vec());
    sqlx::query("UPDATE incidents SET embedding = $1 WHERE id = $2")
        .bind(vec)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn store_handoff_embedding(
    pool: &PgPool,
    id: Uuid,
    embedding: &[f32],
) -> Result<(), sqlx::Error> {
    let vec = Vector::from(embedding.to_vec());
    sqlx::query("UPDATE handoffs SET embedding = $1 WHERE id = $2")
        .bind(vec)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

// ===== HYBRID RRF SEARCH =====
// Combines FTS (websearch_to_tsquery) with vector cosine similarity via Reciprocal Rank Fusion.
// Falls back to FTS-only (with OR relaxation) when query_embedding is None.

pub async fn hybrid_search_knowledge(
    pool: &PgPool,
    query_text: &str,
    query_embedding: Option<&[f32]>,
    limit: i64,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    match query_embedding {
        Some(emb) => {
            let vec = Vector::from(emb.to_vec());
            sqlx::query_as::<_, Knowledge>(
                "WITH fts AS (
                    SELECT id, ROW_NUMBER() OVER (ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC) AS rank
                    FROM knowledge
                    WHERE search_vector @@ websearch_to_tsquery('english', $1)
                    LIMIT 50
                ),
                vec AS (
                    SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <=> $2) AS rank
                    FROM knowledge
                    WHERE embedding IS NOT NULL
                    ORDER BY embedding <=> $2
                    LIMIT 50
                ),
                rrf AS (
                    SELECT COALESCE(f.id, v.id) AS id,
                        COALESCE(1.0 / (60 + f.rank), 0) + COALESCE(1.0 / (60 + v.rank), 0) AS score
                    FROM fts f FULL OUTER JOIN vec v ON f.id = v.id
                    ORDER BY COALESCE(1.0 / (60 + f.rank), 0) + COALESCE(1.0 / (60 + v.rank), 0) DESC
                    LIMIT $3
                )
                SELECT k.* FROM knowledge k JOIN rrf ON k.id = rrf.id ORDER BY rrf.score DESC",
            )
            .bind(query_text)
            .bind(vec)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            let results = sqlx::query_as::<_, Knowledge>(
                "SELECT * FROM knowledge
                 WHERE search_vector @@ websearch_to_tsquery('english', $1)
                 ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
                 LIMIT $2",
            )
            .bind(query_text)
            .bind(limit)
            .fetch_all(pool)
            .await?;

            if results.is_empty() {
                if let Some(or_text) = super::build_or_tsquery_text(query_text) {
                    return sqlx::query_as::<_, Knowledge>(
                        "SELECT * FROM knowledge
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
    }
}

pub async fn hybrid_search_incidents(
    pool: &PgPool,
    query_text: &str,
    query_embedding: Option<&[f32]>,
    limit: i64,
) -> Result<Vec<Incident>, sqlx::Error> {
    match query_embedding {
        Some(emb) => {
            let vec = Vector::from(emb.to_vec());
            sqlx::query_as::<_, Incident>(
                "WITH fts AS (
                    SELECT id, ROW_NUMBER() OVER (ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC) AS rank
                    FROM incidents
                    WHERE search_vector @@ websearch_to_tsquery('english', $1)
                    LIMIT 50
                ),
                vec AS (
                    SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <=> $2) AS rank
                    FROM incidents
                    WHERE embedding IS NOT NULL
                    ORDER BY embedding <=> $2
                    LIMIT 50
                ),
                rrf AS (
                    SELECT COALESCE(f.id, v.id) AS id,
                        COALESCE(1.0 / (60 + f.rank), 0) + COALESCE(1.0 / (60 + v.rank), 0) AS score
                    FROM fts f FULL OUTER JOIN vec v ON f.id = v.id
                    ORDER BY COALESCE(1.0 / (60 + f.rank), 0) + COALESCE(1.0 / (60 + v.rank), 0) DESC
                    LIMIT $3
                )
                SELECT i.* FROM incidents i JOIN rrf ON i.id = rrf.id ORDER BY rrf.score DESC",
            )
            .bind(query_text)
            .bind(vec)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            let results = sqlx::query_as::<_, Incident>(
                "SELECT * FROM incidents
                 WHERE search_vector @@ websearch_to_tsquery('english', $1)
                 ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
                 LIMIT $2",
            )
            .bind(query_text)
            .bind(limit)
            .fetch_all(pool)
            .await?;

            if results.is_empty() {
                if let Some(or_text) = super::build_or_tsquery_text(query_text) {
                    return sqlx::query_as::<_, Incident>(
                        "SELECT * FROM incidents
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
    }
}

pub async fn hybrid_search_handoffs(
    pool: &PgPool,
    query_text: &str,
    query_embedding: Option<&[f32]>,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    match query_embedding {
        Some(emb) => {
            let vec = Vector::from(emb.to_vec());
            sqlx::query_as::<_, Handoff>(
                "WITH fts AS (
                    SELECT id, ROW_NUMBER() OVER (ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC) AS rank
                    FROM handoffs
                    WHERE search_vector @@ websearch_to_tsquery('english', $1)
                    LIMIT 50
                ),
                vec AS (
                    SELECT id, ROW_NUMBER() OVER (ORDER BY embedding <=> $2) AS rank
                    FROM handoffs
                    WHERE embedding IS NOT NULL
                    ORDER BY embedding <=> $2
                    LIMIT 50
                ),
                rrf AS (
                    SELECT COALESCE(f.id, v.id) AS id,
                        COALESCE(1.0 / (60 + f.rank), 0) + COALESCE(1.0 / (60 + v.rank), 0) AS score
                    FROM fts f FULL OUTER JOIN vec v ON f.id = v.id
                    ORDER BY COALESCE(1.0 / (60 + f.rank), 0) + COALESCE(1.0 / (60 + v.rank), 0) DESC
                    LIMIT $3
                )
                SELECT h.* FROM handoffs h JOIN rrf ON h.id = rrf.id ORDER BY rrf.score DESC",
            )
            .bind(query_text)
            .bind(vec)
            .bind(limit)
            .fetch_all(pool)
            .await
        }
        None => {
            let results = sqlx::query_as::<_, Handoff>(
                "SELECT * FROM handoffs
                 WHERE search_vector @@ websearch_to_tsquery('english', $1)
                 ORDER BY ts_rank(search_vector, websearch_to_tsquery('english', $1)) DESC
                 LIMIT $2",
            )
            .bind(query_text)
            .bind(limit)
            .fetch_all(pool)
            .await?;

            if results.is_empty() {
                if let Some(or_text) = super::build_or_tsquery_text(query_text) {
                    return sqlx::query_as::<_, Handoff>(
                        "SELECT * FROM handoffs
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
    }
}

// ===== VECTOR-ONLY SEARCH =====

pub async fn vector_search_knowledge(
    pool: &PgPool,
    query_embedding: &[f32],
    limit: i64,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    let vec = Vector::from(query_embedding.to_vec());
    sqlx::query_as::<_, Knowledge>(
        "SELECT * FROM knowledge WHERE embedding IS NOT NULL ORDER BY embedding <=> $1 LIMIT $2",
    )
    .bind(vec)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Find knowledge entries similar to the given embedding within a cosine distance threshold.
/// Returns (id, title, category, cosine_distance) for duplicate detection.
pub async fn find_similar_knowledge(
    pool: &PgPool,
    query_embedding: &[f32],
    max_distance: f64,
    limit: i64,
) -> Result<Vec<SimilarEntry>, sqlx::Error> {
    let vec = Vector::from(query_embedding.to_vec());
    sqlx::query_as::<_, SimilarEntry>(
        "SELECT id, title, category, (embedding <=> $1)::float8 AS distance
         FROM knowledge
         WHERE embedding IS NOT NULL AND (embedding <=> $1) < $2
         ORDER BY distance
         LIMIT $3",
    )
    .bind(vec)
    .bind(max_distance)
    .bind(limit)
    .fetch_all(pool)
    .await
}

#[derive(Debug, sqlx::FromRow, serde::Serialize)]
pub struct SimilarEntry {
    pub id: Uuid,
    pub title: String,
    pub category: Option<String>,
    pub distance: f64,
}

pub async fn vector_search_incidents(
    pool: &PgPool,
    query_embedding: &[f32],
    limit: i64,
) -> Result<Vec<Incident>, sqlx::Error> {
    let vec = Vector::from(query_embedding.to_vec());
    sqlx::query_as::<_, Incident>(
        "SELECT * FROM incidents WHERE embedding IS NOT NULL ORDER BY embedding <=> $1 LIMIT $2",
    )
    .bind(vec)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Compact view of an open incident matched by semantic similarity. Returned
/// by `find_similar_open_incidents` so `create_incident` can surface "this
/// looks like work already in flight" without sending the full Incident row
/// for every match.
///
/// `cross_client_safe` and `client_id` are included so the cross-client gate
/// in the handler can decide which matches to release vs withhold.
#[derive(Debug, sqlx::FromRow, serde::Serialize)]
pub struct SimilarIncident {
    pub id: Uuid,
    pub title: String,
    pub status: String,
    pub severity: String,
    pub client_id: Option<Uuid>,
    pub cross_client_safe: bool,
    pub created_at: DateTime<Utc>,
    pub distance: f64,
}

/// Find OPEN incidents semantically similar to the given embedding within a
/// cosine distance threshold. Excludes `exclude_id` (the incident that just
/// produced the query embedding, so a freshly-created row never matches itself).
///
/// Returns matches across ALL clients; the cross-client gate is applied at the
/// handler layer (`tools::helpers::filter_cross_client`) so this function stays
/// composable with the standard scoping pattern used elsewhere.
///
/// Used by `create_incident` to surface related work to the caller. Threshold
/// 0.30 (≈70% similarity) is empirical — looser than knowledge's 0.15
/// (duplicate detection) because the goal here is "related work" not "same thing".
pub async fn find_similar_open_incidents(
    pool: &PgPool,
    query_embedding: &[f32],
    exclude_id: Uuid,
    max_distance: f64,
    limit: i64,
) -> Result<Vec<SimilarIncident>, sqlx::Error> {
    let vec = Vector::from(query_embedding.to_vec());
    sqlx::query_as::<_, SimilarIncident>(
        "SELECT id, title, status, severity, client_id, cross_client_safe, created_at,
                (embedding <=> $1)::float8 AS distance
         FROM incidents
         WHERE embedding IS NOT NULL
           AND status = 'open'
           AND id != $2
           AND (embedding <=> $1) < $3
         ORDER BY distance
         LIMIT $4",
    )
    .bind(vec)
    .bind(exclude_id)
    .bind(max_distance)
    .bind(limit)
    .fetch_all(pool)
    .await
}

/// Telemetry helper: cosine distance to the nearest OPEN incident, regardless
/// of threshold. Used by `create_incident` in the "no matches above threshold"
/// branch to log the distance of the nearest-miss, so we can gather a real
/// distribution of distances and retune the 0.30 threshold from data instead
/// of guessing.
///
/// Returns `None` when there are no other open incidents with embeddings
/// (cold start or freshly purged state).
pub async fn nearest_open_incident_distance(
    pool: &PgPool,
    query_embedding: &[f32],
    exclude_id: Uuid,
) -> Result<Option<f64>, sqlx::Error> {
    let vec = Vector::from(query_embedding.to_vec());
    sqlx::query_scalar::<_, f64>(
        "SELECT (embedding <=> $1)::float8 AS distance
         FROM incidents
         WHERE embedding IS NOT NULL
           AND status = 'open'
           AND id != $2
         ORDER BY distance
         LIMIT 1",
    )
    .bind(vec)
    .bind(exclude_id)
    .fetch_optional(pool)
    .await
}

pub async fn vector_search_handoffs(
    pool: &PgPool,
    query_embedding: &[f32],
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    let vec = Vector::from(query_embedding.to_vec());
    sqlx::query_as::<_, Handoff>(
        "SELECT * FROM handoffs WHERE embedding IS NOT NULL ORDER BY embedding <=> $1 LIMIT $2",
    )
    .bind(vec)
    .bind(limit)
    .fetch_all(pool)
    .await
}

// ===== BACKFILL HELPERS =====

#[derive(Debug)]
pub struct MissingEmbeddingCounts {
    pub knowledge: i64,
    pub incidents: i64,
    pub handoffs: i64,
}

pub async fn count_missing_embeddings(
    pool: &PgPool,
) -> Result<MissingEmbeddingCounts, sqlx::Error> {
    let (knowledge, incidents, handoffs) = tokio::try_join!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM knowledge WHERE embedding IS NULL")
            .fetch_one(pool),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM incidents WHERE embedding IS NULL")
            .fetch_one(pool),
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM handoffs WHERE embedding IS NULL")
            .fetch_one(pool),
    )?;
    Ok(MissingEmbeddingCounts {
        knowledge,
        incidents,
        handoffs,
    })
}

pub async fn get_knowledge_without_embeddings(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Knowledge>, sqlx::Error> {
    sqlx::query_as::<_, Knowledge>(
        "SELECT * FROM knowledge WHERE embedding IS NULL ORDER BY created_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn get_incidents_without_embeddings(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Incident>, sqlx::Error> {
    sqlx::query_as::<_, Incident>(
        "SELECT * FROM incidents WHERE embedding IS NULL ORDER BY created_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}

pub async fn get_handoffs_without_embeddings(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<Handoff>, sqlx::Error> {
    sqlx::query_as::<_, Handoff>(
        "SELECT * FROM handoffs WHERE embedding IS NULL ORDER BY created_at LIMIT $1",
    )
    .bind(limit)
    .fetch_all(pool)
    .await
}
