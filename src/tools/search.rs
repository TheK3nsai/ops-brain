use sqlx::PgPool;

/// Operator-only embedding maintenance. Kept out of MCP so normal agents do
/// not pay for or accidentally invoke a database-wide maintenance surface.
pub async fn backfill_embeddings(
    pool: &PgPool,
    client: &crate::embeddings::EmbeddingClient,
    table: Option<&str>,
    batch_size: i64,
) -> anyhow::Result<serde_json::Value> {
    let batch_size = batch_size.clamp(1, 100);
    let tables: Vec<&str> = match table {
        Some(t) => vec![t],
        None => vec!["knowledge", "handoffs"],
    };

    let mut summary = serde_json::Map::new();

    for table in &tables {
        let mut processed = 0i64;

        match *table {
            "knowledge" => {
                let rows =
                    crate::repo::embedding_repo::get_knowledge_without_embeddings(pool, batch_size)
                        .await?;
                if !rows.is_empty() {
                    let texts: Vec<String> = rows
                        .iter()
                        .map(crate::embeddings::prepare_knowledge_text)
                        .collect();
                    let embeddings = client.embed_texts(&texts).await?;
                    for (row, emb) in rows.iter().zip(embeddings.iter()) {
                        crate::repo::embedding_repo::store_knowledge_embedding(pool, row.id, emb)
                            .await?;
                        processed += 1;
                    }
                }
            }
            "handoffs" => {
                let rows =
                    crate::repo::embedding_repo::get_handoffs_without_embeddings(pool, batch_size)
                        .await?;
                if !rows.is_empty() {
                    let texts: Vec<String> = rows
                        .iter()
                        .map(crate::embeddings::prepare_handoff_text)
                        .collect();
                    let embeddings = client.embed_texts(&texts).await?;
                    for (row, emb) in rows.iter().zip(embeddings.iter()) {
                        crate::repo::embedding_repo::store_handoff_embedding(pool, row.id, emb)
                            .await?;
                        processed += 1;
                    }
                }
            }
            _ => anyhow::bail!("unknown table '{table}'; use knowledge or handoffs"),
        }

        summary.insert(
            format!("{table}_processed"),
            serde_json::Value::Number(processed.into()),
        );
    }

    let counts = crate::repo::embedding_repo::count_missing_embeddings(pool).await?;
    summary.insert(
        "remaining_knowledge".to_string(),
        serde_json::Value::Number(counts.knowledge.into()),
    );

    summary.insert(
        "remaining_handoffs".to_string(),
        serde_json::Value::Number(counts.handoffs.into()),
    );

    Ok(serde_json::Value::Object(summary))
}
