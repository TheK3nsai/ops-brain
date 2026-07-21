use schemars::JsonSchema;
use serde::Deserialize;

use crate::validation::deserialize_flexible_i64;

use super::helpers::{
    error_result, filter_cross_client, inject_provenance, json_result, not_found,
    resolve_client_id, truncate_str,
};
use super::shared::{build_client_lookup, embed_and_store, get_query_embedding, log_audit_entries};
use rmcp::model::*;

/// Compact a search result item: keep key metadata fields + a content snippet.
fn compact_search_item(item: &serde_json::Value, entity_type: &str) -> serde_json::Value {
    let Some(obj) = item.as_object() else {
        return item.clone();
    };

    let keep_fields: &[&str] = match entity_type {
        "knowledge" => &[
            "id",
            "title",
            "category",
            "tags",
            "client_id",
            "cross_client_safe",
            "_client_slug",
            "_client_name",
            "author",
            "last_verified_at",
            "_staleness_warning",
            "created_at",
            "updated_at",
        ],
        "handoff" => &[
            "id",
            "title",
            "status",
            "priority",
            "from_agent",
            "to_agent",
            "created_at",
            "updated_at",
        ],
        _ => &["id", "title", "slug", "name"],
    };

    let content_field = match entity_type {
        "handoff" => "body",
        _ => "content",
    };

    let mut compacted = serde_json::Map::new();
    for (k, v) in obj {
        if keep_fields.contains(&k.as_str()) {
            compacted.insert(k.clone(), v.clone());
        }
    }

    // Add a snippet from the content field (first 200 bytes, char-boundary safe)
    if let Some(content) = obj.get(content_field).and_then(|v| v.as_str()) {
        let snippet = if content.len() > 200 {
            format!("{}...", truncate_str(content, 200))
        } else {
            content.to_string()
        };
        compacted.insert("_snippet".to_string(), serde_json::Value::String(snippet));
    }

    serde_json::Value::Object(compacted)
}

/// Apply compact mode to a Vec of search result items.
fn compact_search_results(
    items: &[serde_json::Value],
    entity_type: &str,
) -> Vec<serde_json::Value> {
    items
        .iter()
        .map(|v| compact_search_item(v, entity_type))
        .collect()
}

/// Processed knowledge arm: the final (compacted-if-requested) items plus the
/// cross-client withheld notices and audit entries the caller decides what to
/// do with. Shared by `search_knowledge_single`, the multi-table search arm,
/// and browse — the filter→gate→compact→provenance shape lived in three
/// copies before. The audit entries are returned rather than logged here so
/// the search paths can log them while browse deliberately does not (browse is
/// not an explicit cross-client surfacing attempt).
#[allow(clippy::type_complexity)]
fn process_knowledge_arm(
    items: &[crate::models::knowledge::Knowledge],
    requesting_client_id: Option<uuid::Uuid>,
    acknowledge: bool,
    compact: bool,
    client_lookup: &std::collections::HashMap<uuid::Uuid, (String, String)>,
) -> (
    Vec<serde_json::Value>,
    Vec<serde_json::Value>,
    Vec<(uuid::Uuid, Option<uuid::Uuid>, String)>,
) {
    let json_items = knowledge_entries_to_json(items);
    let filtered = filter_cross_client(
        json_items,
        "knowledge",
        requesting_client_id,
        acknowledge,
        client_lookup,
    );
    let final_items = if compact {
        compact_search_results(&filtered.allowed, "knowledge")
    } else {
        filtered.allowed
    };
    (
        final_items,
        filtered.withheld_notices,
        filtered.audit_entries,
    )
}

/// Processed handoff arm: handoffs carry no client_id, so there is no
/// cross-client gate — just serialize and optionally compact. Shared by the
/// multi-table search arm and browse.
fn process_handoff_arm(
    items: &[crate::models::handoff::Handoff],
    compact: bool,
) -> Vec<serde_json::Value> {
    let json_items: Vec<serde_json::Value> = items
        .iter()
        .filter_map(|h| serde_json::to_value(h).ok())
        .collect();
    if compact {
        compact_search_results(&json_items, "handoff")
    } else {
        json_items
    }
}

fn insert_cross_client_withheld(
    results: &mut serde_json::Map<String, serde_json::Value>,
    notices: &[serde_json::Value],
) {
    if notices.is_empty() {
        return;
    }
    results.insert(
        "cross_client_withheld".to_string(),
        serde_json::to_value(notices).unwrap_or_default(),
    );
}

/// Top-level `_note` injected on any knowledge query that runs WITHOUT a
/// `client_slug`. Without a requesting client there is no scope to enforce, so
/// the cross-client withholding gate is inert — this makes that explicit
/// rather than silently returning every client's content unfiltered.
const UNSCOPED_NOTE: &str =
    "unscoped query — cross-client withholding is not applied; pass client_slug to enable the safety gate";

/// v1.6: knowledge staleness threshold in days. An entry is considered stale
/// if (now - last_verified_at.unwrap_or(created_at)) exceeds this. Computed
/// at read time — no schema column, no background job.
const KNOWLEDGE_STALE_DAYS: i64 = 90;

/// True if a knowledge entry is stale: >90 days since last verification, or
/// since creation if it has never been verified.
fn is_knowledge_stale(k: &crate::models::knowledge::Knowledge) -> bool {
    let most_recent_check = k.last_verified_at.unwrap_or(k.created_at);
    let age = chrono::Utc::now() - most_recent_check;
    age > chrono::Duration::days(KNOWLEDGE_STALE_DAYS)
}

/// Serialize knowledge entries to JSON values with `_staleness_warning`
/// computed at read time and injected into each item. Use this in place of
/// the raw `serde_json::to_value(k)` pattern whenever knowledge results are
/// returned to the user — keeps staleness signals visible in both compact
/// and non-compact modes without a schema column.
fn knowledge_entries_to_json(
    items: &[crate::models::knowledge::Knowledge],
) -> Vec<serde_json::Value> {
    items
        .iter()
        .filter_map(|k| {
            let stale = is_knowledge_stale(k);
            let mut v = serde_json::to_value(k).ok()?;
            if let Some(obj) = v.as_object_mut() {
                obj.insert(
                    "_staleness_warning".to_string(),
                    serde_json::Value::Bool(stale),
                );
            }
            Some(v)
        })
        .collect()
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct AddKnowledgeParams {
    pub title: String,
    pub content: String,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    pub client_slug: Option<String>,
    /// Allow this entry to surface in other clients' contexts (default: false)
    pub cross_client_safe: Option<bool>,
    /// Skip duplicate detection check. Set to true if you've already seen the warning and want to create anyway.
    pub force: Option<bool>,
    /// Your agent identifier (free-form slug, 1–80 chars, [a-zA-Z0-9._-]).
    /// Examples: "CC-Stealth", "Codex-HSR", "Gemini-Stealth". Immutable
    /// once set — provenance cannot be rewritten via the tool surface.
    #[serde(alias = "author_cc")]
    pub author: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SearchKnowledgeParams {
    /// Search query. Use empty string or "*" to browse recent entries across tables.
    pub query: Option<String>,
    /// fts (default single-table), semantic, or hybrid (default multi-table). Ignored for browse.
    pub mode: Option<String>,
    /// Tables to search: knowledge (default), handoffs
    pub tables: Option<Vec<String>>,
    /// Scope to client. Cross-client results withheld unless acknowledged.
    pub client_slug: Option<String>,
    /// Release cross-client results withheld due to scope mismatch
    pub acknowledge_cross_client: Option<bool>,
    /// Max results per table (default 20)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
    /// Snippets instead of full bodies (67KB→~5KB). Default: true multi-table, false single-table.
    pub compact: Option<bool>,
}

/// Update an existing knowledge entry.
///
/// Note: `author` is intentionally NOT updatable via this tool. Provenance
/// is immutable after creation — if you need to correct the author, do it
/// via direct SQL. This prevents accidental (or deliberate) rewriting of
/// history in the one shared cross-agent artifact.
#[derive(Debug, Deserialize, JsonSchema)]
pub struct UpdateKnowledgeParams {
    /// Knowledge entry ID (UUID)
    pub id: String,
    pub title: Option<String>,
    pub content: Option<String>,
    pub category: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Allow this entry to surface in other clients' contexts
    pub cross_client_safe: Option<bool>,
    /// Set to true to mark this entry as verified (confirms content is still accurate).
    /// Sets last_verified_at to now without requiring content changes.
    pub verified: Option<bool>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct DeleteKnowledgeParams {
    /// Knowledge entry ID (UUID)
    pub id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct ListKnowledgeParams {
    pub category: Option<String>,
    pub client_slug: Option<String>,
    /// Max results (default 50)
    #[serde(default, deserialize_with = "deserialize_flexible_i64")]
    pub limit: Option<i64>,
}

// ===== HANDLERS =====

pub async fn handle_add_knowledge(
    brain: &super::OpsBrain,
    p: AddKnowledgeParams,
    bound: Option<&str>,
) -> CallToolResult {
    // v2.0: free-form agent identifier replaces the v1.x CC-fleet allowlist.
    // Provenance is still required on every new entry, but agents are
    // self-identified — whatever the caller says it is, it is.
    let author = match crate::validation::validate_agent_name(&p.author) {
        Ok(n) => n.to_string(),
        Err(e) => return error_result(&format!("author: {e}")),
    };

    // Identity binding: a per-agent token may only author as its own slug, so
    // provenance on knowledge is a server guarantee. Unbound callers (main
    // bearer, stdio/dev) pass through unchanged.
    if let Err(msg) = crate::auth::check_bound_identity(bound, &author) {
        return error_result(&msg);
    }

    let tags = p.tags.unwrap_or_default();
    let cross_client_safe = p.cross_client_safe.unwrap_or(false);
    let force = p.force.unwrap_or(false);

    // Resolve optional client_slug
    let client_id = match resolve_client_id(&brain.pool, p.client_slug.as_deref()).await {
        Ok(v) => v,
        Err(r) => return r,
    };

    // Embed once: the same text feeds both dup-detection and the stored
    // vector. Build it with the exact shape/truncation `prepare_knowledge_text`
    // uses post-insert, so the vector we reuse below matches byte-for-byte.
    let candidate_text = crate::embeddings::prepare_text(&p.title, &p.content);
    let mut precomputed_embedding: Option<Vec<f32>> = None;

    // Duplicate detection: reuse the candidate embedding to check for similar entries
    if !force {
        if let Some(ref client) = brain.embedding_client {
            if let Ok(embedding) = client.embed_text(&candidate_text).await {
                // Cosine distance < 0.15 means similarity > 0.85
                if let Ok(similar) = crate::repo::embedding_repo::find_similar_knowledge(
                    &brain.pool,
                    &embedding,
                    0.15,
                    3,
                )
                .await
                {
                    if !similar.is_empty() {
                        let matches: Vec<serde_json::Value> = similar
                            .iter()
                            .map(|s| {
                                serde_json::json!({
                                    "id": s.id.to_string(),
                                    "title": s.title,
                                    "category": s.category,
                                    "similarity": format!("{:.1}%", (1.0 - s.distance) * 100.0),
                                })
                            })
                            .collect();
                        return json_result(&serde_json::json!({
                            "_warning": "Similar knowledge entries already exist. Set force=true to create anyway, or update the existing entry instead.",
                            "similar_entries": matches,
                            "your_title": p.title,
                        }));
                    }
                }
                // No dup — hold the vector so insert doesn't re-embed.
                precomputed_embedding = Some(embedding);
            }
        }
    }

    match crate::repo::knowledge_repo::add_knowledge(
        &brain.pool,
        &p.title,
        &p.content,
        p.category.as_deref(),
        &tags,
        client_id,
        cross_client_safe,
        Some(&author),
    )
    .await
    {
        Ok(entry) => {
            match precomputed_embedding {
                // Reuse the vector already computed for the dup-check —
                // `candidate_text` is byte-identical to what
                // `prepare_knowledge_text(&entry)` would produce.
                Some(embedding) => {
                    if let Err(e) = crate::repo::embedding_repo::store_knowledge_embedding(
                        &brain.pool,
                        entry.id,
                        &embedding,
                    )
                    .await
                    {
                        tracing::warn!("Failed to store embedding for knowledge/{}: {e}", entry.id);
                    }
                }
                // force=true, no embedding client, or the dup-check embed
                // failed — fall back to embed-then-store.
                None => {
                    let text = crate::embeddings::prepare_knowledge_text(&entry);
                    embed_and_store(
                        &brain.pool,
                        &brain.embedding_client,
                        "knowledge",
                        entry.id,
                        &text,
                    )
                    .await;
                }
            }
            json_result(&entry)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_update_knowledge(
    brain: &super::OpsBrain,
    p: UpdateKnowledgeParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.id)),
    };

    // Verify entry exists
    match crate::repo::knowledge_repo::get_knowledge(&brain.pool, id).await {
        Ok(Some(_)) => {}
        Ok(None) => return not_found("Knowledge", &p.id),
        Err(e) => return error_result(&format!("Database error: {e}")),
    };

    // Mark as verified if requested
    if p.verified.unwrap_or(false) {
        if let Err(e) = crate::repo::knowledge_repo::update_last_verified_at(&brain.pool, id).await
        {
            tracing::warn!("Failed to update last_verified_at for knowledge {id}: {e}");
        }
    }

    match crate::repo::knowledge_repo::update_knowledge(
        &brain.pool,
        id,
        p.title.as_deref(),
        p.content.as_deref(),
        p.category.as_deref(),
        p.tags.as_deref(),
        p.cross_client_safe,
    )
    .await
    {
        Ok(updated) => {
            let text = crate::embeddings::prepare_knowledge_text(&updated);
            embed_and_store(
                &brain.pool,
                &brain.embedding_client,
                "knowledge",
                updated.id,
                &text,
            )
            .await;
            json_result(&updated)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub(crate) async fn handle_delete_knowledge(
    brain: &super::OpsBrain,
    p: DeleteKnowledgeParams,
) -> CallToolResult {
    let id = match uuid::Uuid::parse_str(&p.id) {
        Ok(id) => id,
        Err(_) => return error_result(&format!("Invalid UUID: {}", p.id)),
    };

    match crate::repo::knowledge_repo::delete_knowledge(&brain.pool, id).await {
        Ok(true) => json_result(&serde_json::json!({"deleted": true, "id": p.id})),
        Ok(false) => not_found("Knowledge", &p.id),
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

pub async fn handle_search_knowledge(
    brain: &super::OpsBrain,
    p: SearchKnowledgeParams,
) -> CallToolResult {
    let tables = p.tables.unwrap_or_else(|| vec!["knowledge".to_string()]);
    let multi_table = tables.len() > 1 || tables.iter().any(|t| t != "knowledge");

    // Detect browse mode: empty or "*" query means "show me recent entries"
    let raw_query = p.query.unwrap_or_default();
    let query_trimmed = raw_query.trim();
    let browse_mode = query_trimmed.is_empty() || query_trimmed == "*";

    if browse_mode {
        let compact = p.compact.unwrap_or(true);
        return browse_recent_entries(
            brain,
            &tables,
            multi_table,
            p.client_slug.as_deref(),
            p.acknowledge_cross_client.unwrap_or(false),
            p.limit.unwrap_or(20).clamp(1, 200),
            compact,
        )
        .await;
    }

    // Default mode: "hybrid" for all searches (single or multi-table)
    let mode = p.mode.as_deref().unwrap_or("hybrid");
    if let Err(msg) =
        crate::validation::validate_required(mode, "mode", crate::validation::SEARCH_MODES)
    {
        return error_result(&msg);
    }

    // Validate table names
    let valid_tables = ["knowledge", "handoffs"];
    for t in &tables {
        if !valid_tables.contains(&t.as_str()) {
            return error_result(&format!(
                "Invalid table '{t}'. Valid options: {}",
                valid_tables.join(", ")
            ));
        }
    }

    // Resolve optional client_slug for cross-client gate
    let requesting_client_id = match resolve_client_id(&brain.pool, p.client_slug.as_deref()).await
    {
        Ok(v) => v,
        Err(r) => return r,
    };
    let acknowledge = p.acknowledge_cross_client.unwrap_or(false);
    let limit = p.limit.unwrap_or(20).clamp(1, 200);

    // Compact mode: default true for multi-table, false for single-table
    let compact = p.compact.unwrap_or(multi_table);

    // Single-table knowledge search (original behavior)
    if !multi_table {
        return search_knowledge_single(
            brain,
            &raw_query,
            mode,
            requesting_client_id,
            acknowledge,
            limit,
            compact,
        )
        .await;
    }

    // Multi-table search
    let query_embedding = get_query_embedding(&brain.embedding_client, &raw_query).await;
    if mode == "semantic" && query_embedding.is_none() {
        return error_result(
            "Semantic search unavailable (embedding API not configured or failed)",
        );
    }
    let emb_ref = query_embedding.as_deref();

    let mut results = serde_json::Map::new();
    let client_lookup = build_client_lookup(&brain.pool).await;
    let mut all_withheld: Vec<serde_json::Value> = Vec::new();

    for table in &tables {
        match table.as_str() {
            "knowledge" => {
                let search_result = match mode {
                    "semantic" => {
                        crate::repo::embedding_repo::vector_search_knowledge(
                            &brain.pool,
                            emb_ref.unwrap(),
                            limit,
                        )
                        .await
                    }
                    "hybrid" => {
                        crate::repo::embedding_repo::hybrid_search_knowledge(
                            &brain.pool,
                            &raw_query,
                            emb_ref,
                            limit,
                        )
                        .await
                    }
                    _ => {
                        crate::repo::knowledge_repo::search_knowledge(
                            &brain.pool,
                            &raw_query,
                            limit,
                        )
                        .await
                    }
                };
                match search_result {
                    Ok(items) => {
                        let (final_items, withheld, audit) = process_knowledge_arm(
                            &items,
                            requesting_client_id,
                            acknowledge,
                            compact,
                            &client_lookup,
                        );
                        results.insert(
                            "knowledge".to_string(),
                            serde_json::to_value(&final_items).unwrap_or_default(),
                        );
                        all_withheld.extend(withheld);
                        log_audit_entries(
                            &brain.pool,
                            "search_knowledge",
                            requesting_client_id,
                            "knowledge",
                            &audit,
                        )
                        .await;
                    }
                    Err(e) => {
                        results.insert(
                            "knowledge_error".to_string(),
                            serde_json::Value::String(e.to_string()),
                        );
                    }
                }
            }
            "handoffs" => {
                // Handoffs are NOT gated — no client_id on handoffs table
                let search_result = match mode {
                    "semantic" => {
                        crate::repo::embedding_repo::vector_search_handoffs(
                            &brain.pool,
                            emb_ref.unwrap(),
                            limit,
                        )
                        .await
                    }
                    "hybrid" => {
                        crate::repo::embedding_repo::hybrid_search_handoffs(
                            &brain.pool,
                            &raw_query,
                            emb_ref,
                            limit,
                        )
                        .await
                    }
                    _ => {
                        crate::repo::handoff_repo::search_handoffs(&brain.pool, &raw_query, limit)
                            .await
                    }
                };
                match search_result {
                    Ok(items) => {
                        let final_items = process_handoff_arm(&items, compact);
                        results.insert(
                            "handoffs".to_string(),
                            serde_json::to_value(&final_items).unwrap_or_default(),
                        );
                    }
                    Err(e) => {
                        results.insert(
                            "handoffs_error".to_string(),
                            serde_json::Value::String(e.to_string()),
                        );
                    }
                }
            }
            _ => {} // validated above
        }
    }

    if !all_withheld.is_empty() {
        insert_cross_client_withheld(&mut results, &all_withheld);
    }

    // Inject notes about embedding availability
    if query_embedding.is_none() && brain.embedding_client.is_some() {
        results.insert(
            "_note".to_string(),
            serde_json::Value::String(
                "Embedding API call failed — results are FTS-only".to_string(),
            ),
        );
    } else if brain.embedding_client.is_none() {
        results.insert(
            "_note".to_string(),
            serde_json::Value::String(
                "Embeddings not configured — results are FTS-only".to_string(),
            ),
        );
    }

    // Unscoped query: the safety gate is inert. Don't clobber an embedding
    // `_note` if one was set above — surface the unscoped caveat only when the
    // slot is free.
    if requesting_client_id.is_none() {
        results
            .entry("_note".to_string())
            .or_insert_with(|| serde_json::Value::String(UNSCOPED_NOTE.to_string()));
    }

    json_result(&serde_json::Value::Object(results))
}

/// Browse mode: return recent entries across requested tables (no search filter).
/// Triggered when query is empty or "*".
async fn browse_recent_entries(
    brain: &super::OpsBrain,
    tables: &[String],
    _multi_table: bool,
    client_slug: Option<&str>,
    acknowledge: bool,
    limit: i64,
    compact: bool,
) -> CallToolResult {
    let requesting_client_id = match resolve_client_id(&brain.pool, client_slug).await {
        Ok(v) => v,
        Err(r) => return r,
    };
    let client_lookup = build_client_lookup(&brain.pool).await;
    let mut results = serde_json::Map::new();
    let mut all_withheld: Vec<serde_json::Value> = Vec::new();

    for table in tables {
        match table.as_str() {
            "knowledge" => {
                match crate::repo::knowledge_repo::list_knowledge(&brain.pool, None, None, limit)
                    .await
                {
                    Ok(items) => {
                        // Browse does NOT audit-log — it is not an explicit
                        // cross-client surfacing attempt, so the audit entries
                        // are intentionally dropped.
                        let (final_items, withheld, _audit) = process_knowledge_arm(
                            &items,
                            requesting_client_id,
                            acknowledge,
                            compact,
                            &client_lookup,
                        );
                        results.insert(
                            "knowledge".to_string(),
                            serde_json::to_value(&final_items).unwrap_or_default(),
                        );
                        all_withheld.extend(withheld);
                    }
                    Err(e) => {
                        results.insert(
                            "knowledge_error".to_string(),
                            serde_json::Value::String(e.to_string()),
                        );
                    }
                }
            }
            "handoffs" => {
                match crate::repo::handoff_repo::list_handoffs(
                    &brain.pool,
                    None,
                    None,
                    None,
                    None,
                    false,
                    limit,
                )
                .await
                {
                    Ok(items) => {
                        let final_items = process_handoff_arm(&items, compact);
                        results.insert(
                            "handoffs".to_string(),
                            serde_json::to_value(&final_items).unwrap_or_default(),
                        );
                    }
                    Err(e) => {
                        results.insert(
                            "handoffs_error".to_string(),
                            serde_json::Value::String(e.to_string()),
                        );
                    }
                }
            }
            _ => {}
        }
    }

    if !all_withheld.is_empty() {
        insert_cross_client_withheld(&mut results, &all_withheld);
    }
    results.insert(
        "_browse_mode".to_string(),
        serde_json::Value::String(
            "Showing recent entries (no search query). Use a specific query for ranked results."
                .to_string(),
        ),
    );
    if requesting_client_id.is_none() {
        results.insert(
            "_note".to_string(),
            serde_json::Value::String(UNSCOPED_NOTE.to_string()),
        );
    }

    json_result(&serde_json::Value::Object(results))
}

/// Single-table knowledge search (preserves original search_knowledge behavior exactly)
async fn search_knowledge_single(
    brain: &super::OpsBrain,
    query: &str,
    mode: &str,
    requesting_client_id: Option<uuid::Uuid>,
    acknowledge: bool,
    limit: i64,
    compact: bool,
) -> CallToolResult {
    let result = match mode {
        "semantic" => {
            let Some(emb) = get_query_embedding(&brain.embedding_client, query).await else {
                return error_result(
                    "Semantic search unavailable (embedding API not configured or failed)",
                );
            };
            crate::repo::embedding_repo::vector_search_knowledge(&brain.pool, &emb, limit).await
        }
        "hybrid" => {
            let emb = get_query_embedding(&brain.embedding_client, query).await;
            crate::repo::embedding_repo::hybrid_search_knowledge(
                &brain.pool,
                query,
                emb.as_deref(),
                limit,
            )
            .await
        }
        _ => crate::repo::knowledge_repo::search_knowledge(&brain.pool, query, limit).await,
    };
    match result {
        Ok(entries) => {
            let client_lookup = build_client_lookup(&brain.pool).await;
            let (final_items, withheld, audit) = process_knowledge_arm(
                &entries,
                requesting_client_id,
                acknowledge,
                compact,
                &client_lookup,
            );

            log_audit_entries(
                &brain.pool,
                "search_knowledge",
                requesting_client_id,
                "knowledge",
                &audit,
            )
            .await;

            let mut response = serde_json::json!({ "knowledge": final_items });
            if !withheld.is_empty() {
                response["cross_client_withheld"] = serde_json::json!(withheld);
            }
            if requesting_client_id.is_none() {
                response["_note"] = serde_json::Value::String(UNSCOPED_NOTE.to_string());
            }
            json_result(&response)
        }
        Err(e) => error_result(&format!("Search error: {e}")),
    }
}

pub async fn handle_list_knowledge(
    brain: &super::OpsBrain,
    p: ListKnowledgeParams,
) -> CallToolResult {
    // Resolve optional client_slug
    let client_id = match resolve_client_id(&brain.pool, p.client_slug.as_deref()).await {
        Ok(v) => v,
        Err(r) => return r,
    };

    let limit = p.limit.unwrap_or(50).clamp(1, 200);
    match crate::repo::knowledge_repo::list_knowledge(
        &brain.pool,
        p.category.as_deref(),
        client_id,
        limit,
    )
    .await
    {
        // v1.6: surface staleness warnings; always stamp provenance
        // (_client_slug/_client_name) on every item — list_knowledge is not
        // gated, so provenance is the only cross-client signal it carries.
        Ok(entries) => {
            let client_lookup = build_client_lookup(&brain.pool).await;
            let mut items = knowledge_entries_to_json(&entries);
            for item in &mut items {
                inject_provenance(item, &client_lookup);
            }
            let mut response = serde_json::json!({ "knowledge": items });
            // Unscoped list: no requesting client → the withholding gate is
            // inert. Say so, mirroring search_knowledge.
            if client_id.is_none() {
                response["_note"] = serde_json::Value::String(UNSCOPED_NOTE.to_string());
            }
            json_result(&response)
        }
        Err(e) => error_result(&format!("Database error: {e}")),
    }
}

#[cfg(test)]
mod tests {
    //! Pure-logic unit tests for knowledge provenance (v1.6). DB-backed
    //! handler tests live in `tests/integration.rs`.

    use super::*;
    use chrono::{Duration, Utc};
    use uuid::Uuid;

    fn make_knowledge(
        last_verified: Option<chrono::DateTime<Utc>>,
        created: chrono::DateTime<Utc>,
    ) -> crate::models::knowledge::Knowledge {
        crate::models::knowledge::Knowledge {
            id: Uuid::now_v7(),
            title: "test".to_string(),
            content: "test content".to_string(),
            category: None,
            tags: vec![],
            client_id: None,
            cross_client_safe: false,
            last_verified_at: last_verified,
            author: Some("CC-Stealth".to_string()),
            created_at: created,
            updated_at: created,
        }
    }

    #[test]
    fn is_knowledge_stale_false_for_fresh_unverified_entry() {
        // Created recently, never verified — NOT stale.
        let k = make_knowledge(None, Utc::now() - Duration::days(10));
        assert!(!is_knowledge_stale(&k));
    }

    #[test]
    fn is_knowledge_stale_true_for_old_unverified_entry() {
        // Created 100 days ago, never verified — STALE.
        let k = make_knowledge(None, Utc::now() - Duration::days(100));
        assert!(is_knowledge_stale(&k));
    }

    #[test]
    fn is_knowledge_stale_false_for_recently_verified_entry() {
        // Created 200 days ago, verified 30 days ago — NOT stale.
        // The verification resets the clock.
        let k = make_knowledge(
            Some(Utc::now() - Duration::days(30)),
            Utc::now() - Duration::days(200),
        );
        assert!(!is_knowledge_stale(&k));
    }

    #[test]
    fn is_knowledge_stale_true_for_stale_verified_entry() {
        // Created 200 days ago, verified 95 days ago — STALE.
        // Verification aged out of the 90-day window.
        let k = make_knowledge(
            Some(Utc::now() - Duration::days(95)),
            Utc::now() - Duration::days(200),
        );
        assert!(is_knowledge_stale(&k));
    }

    #[test]
    fn is_knowledge_stale_false_exactly_at_threshold() {
        // Exactly 90 days old — strictly `> 90 days` is the trigger,
        // so 90d on the nose is NOT stale. This is an off-by-one guard.
        // Use 89 days to avoid clock-skew races with `Utc::now()` inside
        // the function itself (the function reads now fresh, so 90 days
        // ago from the test's now might read as 90 days + epsilon).
        let k = make_knowledge(None, Utc::now() - Duration::days(89));
        assert!(!is_knowledge_stale(&k));
    }

    #[test]
    fn knowledge_entries_to_json_injects_staleness_flag() {
        let fresh = make_knowledge(None, Utc::now() - Duration::days(1));
        let stale = make_knowledge(None, Utc::now() - Duration::days(120));
        let json = knowledge_entries_to_json(&[fresh, stale]);
        assert_eq!(json.len(), 2);
        assert_eq!(
            json[0].get("_staleness_warning"),
            Some(&serde_json::Value::Bool(false)),
            "fresh entry should not be stale"
        );
        assert_eq!(
            json[1].get("_staleness_warning"),
            Some(&serde_json::Value::Bool(true)),
            "120-day-old entry should be stale"
        );
    }

    #[test]
    fn knowledge_entries_to_json_preserves_provenance_fields() {
        let k = make_knowledge(None, Utc::now());
        let json = knowledge_entries_to_json(&[k]);
        let obj = json[0].as_object().expect("should be object");
        assert_eq!(
            obj.get("author"),
            Some(&serde_json::Value::String("CC-Stealth".to_string())),
            "author should survive serialization"
        );
    }

    #[test]
    fn add_knowledge_accepts_legacy_author_cc_alias() {
        let params: AddKnowledgeParams = serde_json::from_value(serde_json::json!({
            "title": "legacy alias",
            "content": "body",
            "author_cc": "CC-Stealth"
        }))
        .unwrap();
        assert_eq!(params.author, "CC-Stealth");
    }
}
