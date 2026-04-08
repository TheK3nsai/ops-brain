//! Integration tests for ops-brain.
//!
//! Requires a running PostgreSQL instance. Uses DATABASE_URL from environment
//! or defaults to the test database. Tests use UUID-based unique slugs for
//! isolation and clean up their own data on success. See common.rs for details.
//!
//! Run: DATABASE_URL=postgres://ops_brain:ops_brain@localhost:5432/ops_brain cargo test

mod common;

use sqlx::PgPool;
use uuid::Uuid;

async fn pool() -> PgPool {
    common::test_pool().await
}

// ===== Client Repo =====

mod client_tests {
    use super::*;

    #[tokio::test]
    async fn upsert_and_get_client() {
        let pool = pool().await;
        let slug = format!("test-client-{}", Uuid::now_v7());

        let client = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Test Client",
            &slug,
            Some("test notes"),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(client.name, "Test Client");
        assert_eq!(client.slug, slug);
        assert_eq!(client.notes.as_deref(), Some("test notes"));

        // Get by slug
        let fetched = ops_brain::repo::client_repo::get_client_by_slug(&pool, &slug)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, client.id);

        // Get by ID
        let fetched = ops_brain::repo::client_repo::get_client(&pool, client.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.slug, slug);

        // Cleanup
        sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(client.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn upsert_client_updates_on_conflict() {
        let pool = pool().await;
        let slug = format!("upsert-test-{}", Uuid::now_v7());

        let _c1 = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Original",
            &slug,
            Some("v1"),
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let c2 = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Updated",
            &slug,
            Some("v2"),
            Some(10),
            None,
            None,
        )
        .await
        .unwrap();

        // Same slug, should update name and notes
        assert_eq!(c2.slug, slug);
        assert_eq!(c2.name, "Updated");
        assert_eq!(c2.notes.as_deref(), Some("v2"));
        assert_eq!(c2.zammad_org_id, Some(10));

        // Cleanup
        sqlx::query("DELETE FROM clients WHERE slug = $1")
            .bind(&slug)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_clients() {
        let pool = pool().await;
        let clients = ops_brain::repo::client_repo::list_clients(&pool)
            .await
            .unwrap();
        // Should at least return without error (may have seed data)
        let _ = clients.len();
    }
}

// ===== Runbook Repo =====

mod runbook_tests {
    use super::*;

    #[tokio::test]
    async fn create_and_get_runbook() {
        let pool = pool().await;
        let slug = format!("test-runbook-{}", Uuid::now_v7());

        let runbook = ops_brain::repo::runbook_repo::create_runbook(
            &pool,
            "Test Runbook",
            &slug,
            Some("testing"),
            "Step 1: Do things",
            &["test".to_string(), "ci".to_string()],
            Some(15),
            false,
            Some("Test notes"),
            None,  // no client
            false, // not cross_client_safe
            None,  // no source_url
        )
        .await
        .unwrap();

        assert_eq!(runbook.title, "Test Runbook");
        assert_eq!(runbook.slug, slug);
        assert_eq!(runbook.category.as_deref(), Some("testing"));
        assert_eq!(runbook.tags, vec!["test", "ci"]);
        assert_eq!(runbook.estimated_minutes, Some(15));
        assert!(!runbook.requires_reboot);
        assert!(!runbook.cross_client_safe);
        assert!(runbook.client_id.is_none());

        // Get by slug
        let fetched = ops_brain::repo::runbook_repo::get_runbook_by_slug(&pool, &slug)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, runbook.id);

        // Cleanup
        sqlx::query("DELETE FROM runbooks WHERE id = $1")
            .bind(runbook.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn create_client_scoped_runbook() {
        let pool = pool().await;
        let client_slug = format!("rb-client-{}", Uuid::now_v7());
        let rb_slug = format!("scoped-rb-{}", Uuid::now_v7());

        let client = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Test Client",
            &client_slug,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let runbook = ops_brain::repo::runbook_repo::create_runbook(
            &pool,
            "Client-Specific Runbook",
            &rb_slug,
            None,
            "Content for this client only",
            &[],
            None,
            false,
            None,
            Some(client.id),
            false,
            None,
        )
        .await
        .unwrap();

        assert_eq!(runbook.client_id, Some(client.id));
        assert!(!runbook.cross_client_safe);

        // list_runbooks with client filter should include this + global
        let list = ops_brain::repo::runbook_repo::list_runbooks(
            &pool,
            None,
            None,
            None,
            None,
            Some(client.id),
            50,
        )
        .await
        .unwrap();

        assert!(list.iter().any(|r| r.id == runbook.id));

        // Cleanup
        sqlx::query("DELETE FROM runbooks WHERE id = $1")
            .bind(runbook.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(client.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn update_runbook_cross_client_safe() {
        let pool = pool().await;
        let slug = format!("update-safe-{}", Uuid::now_v7());

        let runbook = ops_brain::repo::runbook_repo::create_runbook(
            &pool,
            "Runbook To Update",
            &slug,
            None,
            "Original content",
            &[],
            None,
            false,
            None,
            None,
            false,
            None,
        )
        .await
        .unwrap();
        assert!(!runbook.cross_client_safe);

        let updated = ops_brain::repo::runbook_repo::update_runbook(
            &pool,
            runbook.id,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(true),
            None,
        )
        .await
        .unwrap();

        assert!(updated.cross_client_safe);
        assert_eq!(updated.version, 2); // version bumped

        // Cleanup
        sqlx::query("DELETE FROM runbooks WHERE id = $1")
            .bind(runbook.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Knowledge Repo =====

mod knowledge_tests {
    use super::*;

    #[tokio::test]
    async fn add_and_get_knowledge() {
        let pool = pool().await;

        let k = ops_brain::repo::knowledge_repo::add_knowledge(
            &pool,
            "Test Knowledge Entry",
            "Some important info about testing",
            Some("testing"),
            &["ci".to_string()],
            None,
            false,
            Some("CC-Stealth"),
            None,
        )
        .await
        .unwrap();

        assert_eq!(k.title, "Test Knowledge Entry");
        assert!(!k.cross_client_safe);
        assert_eq!(k.author_cc.as_deref(), Some("CC-Stealth"));
        assert_eq!(k.source_incident_id, None);

        let fetched = ops_brain::repo::knowledge_repo::get_knowledge(&pool, k.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, k.id);

        // Cleanup
        sqlx::query("DELETE FROM knowledge WHERE id = $1")
            .bind(k.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn knowledge_cross_client_safe_field() {
        let pool = pool().await;

        let k = ops_brain::repo::knowledge_repo::add_knowledge(
            &pool,
            "Safe Knowledge",
            "Content safe for all clients",
            None,
            &[],
            None,
            true,
            Some("CC-Stealth"),
            None,
        )
        .await
        .unwrap();

        assert!(k.cross_client_safe);

        // Cleanup
        sqlx::query("DELETE FROM knowledge WHERE id = $1")
            .bind(k.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Knowledge Provenance (v1.6) =====
//
// Handler-level end-to-end tests for the provenance feature:
//   - author_cc allowlist validation (fail loudly on invalid names)
//   - source_incident_id FK resolution (exist-check before INSERT)
//   - author_cc immutability across updates (enforced at type level)
//   - source_incident_id updatable post-hoc
//   - staleness flag surfaced in list_knowledge results
//
// Pure logic (is_knowledge_stale, knowledge_entries_to_json) is unit-tested
// in src/tools/knowledge.rs — these tests exist to catch handler-layer
// regressions (validation, error messages, response shapes).

mod knowledge_provenance_tests {
    use super::*;

    fn build_brain(pool: PgPool) -> ops_brain::tools::OpsBrain {
        ops_brain::tools::OpsBrain::new(pool, vec![], None, None)
    }

    fn extract_text(result: &rmcp::model::CallToolResult) -> String {
        result
            .content
            .first()
            .expect("result has at least one content item")
            .as_text()
            .expect("content is text")
            .text
            .clone()
    }

    /// Strip all whitespace so assertions are robust against pretty vs
    /// compact JSON formatting. `json_result` currently pretty-prints,
    /// meaning `"field": "value"` with a space after the colon, but we
    /// don't want tests to break if that changes.
    fn no_ws(s: &str) -> String {
        s.chars().filter(|c| !c.is_whitespace()).collect()
    }

    async fn seed_incident(pool: &PgPool) -> uuid::Uuid {
        let incident = ops_brain::repo::incident_repo::create_incident(
            pool,
            "provenance test incident",
            "low",
            None,
            Some("synthetic symptoms for FK link"),
            Some("created by knowledge_provenance_tests"),
            false,
        )
        .await
        .unwrap();
        incident.id
    }

    #[tokio::test]
    async fn handler_add_knowledge_rejects_invalid_author_cc() {
        let brain = build_brain(pool().await);
        let result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: "rejected title".to_string(),
                content: "rejected content".to_string(),
                category: None,
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-NotReal".to_string(),
                source_incident_id: None,
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(
            text.contains("Invalid author_cc"),
            "error should name the field"
        );
        assert!(text.contains("CC-Stealth"), "error should list valid names");
    }

    #[tokio::test]
    async fn handler_add_knowledge_stores_author_cc() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: format!("provenance stored {}", uuid::Uuid::now_v7()),
                content: "content stamped with author".to_string(),
                category: None,
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-Stealth".to_string(),
                source_incident_id: None,
            },
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        let compact = no_ws(&text);
        assert!(compact.contains("\"author_cc\":\"CC-Stealth\""));
        assert!(compact.contains("\"source_incident_id\":null"));

        // Cleanup: extract id from the JSON and delete.
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let id = v.get("id").and_then(|x| x.as_str()).unwrap().to_string();
        sqlx::query("DELETE FROM knowledge WHERE id = $1::uuid")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn handler_add_knowledge_links_source_incident_id() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let incident_id = seed_incident(&pool).await;

        let result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: format!("linked to incident {incident_id}"),
                content: "knowledge born from an incident".to_string(),
                category: None,
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-Stealth".to_string(),
                source_incident_id: Some(incident_id.to_string()),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        assert!(
            no_ws(&text).contains(&format!("\"source_incident_id\":\"{incident_id}\"")),
            "response should include the FK value"
        );

        // Cleanup (knowledge first, incident last — FK order doesn't matter
        // due to ON DELETE SET NULL, but keeps the fixture tidy).
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let id = v.get("id").and_then(|x| x.as_str()).unwrap().to_string();
        sqlx::query("DELETE FROM knowledge WHERE id = $1::uuid")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM incidents WHERE id = $1")
            .bind(incident_id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn handler_add_knowledge_rejects_nonexistent_incident() {
        let brain = build_brain(pool().await);
        let ghost_id = uuid::Uuid::now_v7();
        let result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: "should not land".to_string(),
                content: "no incident behind this".to_string(),
                category: None,
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-Stealth".to_string(),
                source_incident_id: Some(ghost_id.to_string()),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(
            text.contains("Incident not found"),
            "error should be explicit about the missing incident, not a raw FK violation"
        );
    }

    #[tokio::test]
    async fn handler_add_knowledge_rejects_malformed_incident_uuid() {
        let brain = build_brain(pool().await);
        let result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: "malformed uuid".to_string(),
                content: "not a uuid string".to_string(),
                category: None,
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-Stealth".to_string(),
                source_incident_id: Some("obviously-not-a-uuid".to_string()),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(text.contains("Invalid source_incident_id UUID"));
    }

    #[tokio::test]
    async fn handler_update_knowledge_can_link_source_incident_id_post_hoc() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());

        // Create knowledge without a source incident.
        let add_result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: format!("post-hoc link {}", uuid::Uuid::now_v7()),
                content: "initially unlinked".to_string(),
                category: None,
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-Stealth".to_string(),
                source_incident_id: None,
            },
        )
        .await;
        let text = extract_text(&add_result);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let knowledge_id = v.get("id").and_then(|x| x.as_str()).unwrap().to_string();

        // Seed an incident and link it via update.
        let incident_id = seed_incident(&pool).await;
        let update_result = ops_brain::tools::knowledge::handle_update_knowledge(
            &brain,
            ops_brain::tools::knowledge::UpdateKnowledgeParams {
                id: knowledge_id.clone(),
                title: None,
                content: None,
                category: None,
                tags: None,
                cross_client_safe: None,
                verified: None,
                source_incident_id: Some(incident_id.to_string()),
            },
        )
        .await;
        assert_eq!(update_result.is_error, Some(false));
        let updated_text = extract_text(&update_result);
        let updated_compact = no_ws(&updated_text);
        assert!(updated_compact.contains(&format!("\"source_incident_id\":\"{incident_id}\"")));
        assert!(
            updated_compact.contains("\"author_cc\":\"CC-Stealth\""),
            "author_cc should be preserved across update"
        );

        // Cleanup
        sqlx::query("DELETE FROM knowledge WHERE id = $1::uuid")
            .bind(&knowledge_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM incidents WHERE id = $1")
            .bind(incident_id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn handler_update_knowledge_cannot_mutate_author_cc() {
        // Structural test: UpdateKnowledgeParams has no `author_cc` field,
        // so the compiler itself guarantees this invariant. This test
        // documents the guarantee via a positive assertion: after an
        // update, author_cc is unchanged from the original value.
        let pool = pool().await;
        let brain = build_brain(pool.clone());

        let add_result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: format!("immutable author {}", uuid::Uuid::now_v7()),
                content: "v1".to_string(),
                category: None,
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-HSR".to_string(),
                source_incident_id: None,
            },
        )
        .await;
        let text = extract_text(&add_result);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let knowledge_id = v.get("id").and_then(|x| x.as_str()).unwrap().to_string();
        assert!(no_ws(&text).contains("\"author_cc\":\"CC-HSR\""));

        // Update unrelated field (content) — author_cc must still be CC-HSR.
        let update_result = ops_brain::tools::knowledge::handle_update_knowledge(
            &brain,
            ops_brain::tools::knowledge::UpdateKnowledgeParams {
                id: knowledge_id.clone(),
                title: None,
                content: Some("v2 — rewritten but still by CC-HSR".to_string()),
                category: None,
                tags: None,
                cross_client_safe: None,
                verified: None,
                source_incident_id: None,
            },
        )
        .await;
        assert_eq!(update_result.is_error, Some(false));
        let updated_text = extract_text(&update_result);
        assert!(
            no_ws(&updated_text).contains("\"author_cc\":\"CC-HSR\""),
            "author_cc must be preserved across updates — provenance is immutable"
        );

        // Cleanup
        sqlx::query("DELETE FROM knowledge WHERE id = $1::uuid")
            .bind(&knowledge_id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn handler_list_knowledge_surfaces_staleness_warning() {
        // Create a fresh entry via handler, then backdate it via SQL to
        // force the stale flag, then list and verify the flag is true.
        let pool = pool().await;
        let brain = build_brain(pool.clone());

        let add_result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: format!("staleness test {}", uuid::Uuid::now_v7()),
                content: "will be backdated".to_string(),
                category: Some("provenance-test-stale".to_string()),
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-Stealth".to_string(),
                source_incident_id: None,
            },
        )
        .await;
        let text = extract_text(&add_result);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let knowledge_id = v.get("id").and_then(|x| x.as_str()).unwrap().to_string();

        // Backdate created_at to 100 days ago; leave last_verified_at NULL.
        sqlx::query(
            "UPDATE knowledge SET created_at = now() - interval '100 days', \
             last_verified_at = NULL WHERE id = $1::uuid",
        )
        .bind(&knowledge_id)
        .execute(&pool)
        .await
        .unwrap();

        let list_result = ops_brain::tools::knowledge::handle_list_knowledge(
            &brain,
            ops_brain::tools::knowledge::ListKnowledgeParams {
                category: Some("provenance-test-stale".to_string()),
                client_slug: None,
                limit: Some(10),
            },
        )
        .await;
        assert_eq!(list_result.is_error, Some(false));
        let list_text = extract_text(&list_result);
        assert!(
            no_ws(&list_text).contains("\"_staleness_warning\":true"),
            "backdated entry should be flagged stale; got: {list_text}"
        );

        // Also assert that a fresh entry in the same category comes back false.
        let fresh_add = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: format!("fresh staleness test {}", uuid::Uuid::now_v7()),
                content: "not backdated".to_string(),
                category: Some("provenance-test-stale".to_string()),
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-Stealth".to_string(),
                source_incident_id: None,
            },
        )
        .await;
        let fresh_text = extract_text(&fresh_add);
        let fresh_v: serde_json::Value = serde_json::from_str(&fresh_text).unwrap();
        let fresh_id = fresh_v
            .get("id")
            .and_then(|x| x.as_str())
            .unwrap()
            .to_string();

        let list_again = ops_brain::tools::knowledge::handle_list_knowledge(
            &brain,
            ops_brain::tools::knowledge::ListKnowledgeParams {
                category: Some("provenance-test-stale".to_string()),
                client_slug: None,
                limit: Some(10),
            },
        )
        .await;
        let list_again_text = extract_text(&list_again);
        assert!(
            no_ws(&list_again_text).contains("\"_staleness_warning\":false"),
            "fresh entry should not be flagged stale; got: {list_again_text}"
        );

        // Cleanup
        sqlx::query("DELETE FROM knowledge WHERE id = $1::uuid")
            .bind(&knowledge_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM knowledge WHERE id = $1::uuid")
            .bind(&fresh_id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn source_incident_id_on_delete_set_null() {
        // Migration 20260408000001 uses `ON DELETE SET NULL` on the
        // source_incident_id FK so cleaning up incidents does not
        // cascade-delete the knowledge we learned from them. This test
        // guards the cascade behavior at runtime — if a future migration
        // ever flips this to `ON DELETE CASCADE` by mistake, this test
        // will fail loudly.
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let incident_id = seed_incident(&pool).await;

        let add_result = ops_brain::tools::knowledge::handle_add_knowledge(
            &brain,
            ops_brain::tools::knowledge::AddKnowledgeParams {
                title: format!("cascade test {}", uuid::Uuid::now_v7()),
                content: "linked to an incident that will be deleted".to_string(),
                category: None,
                tags: None,
                client_slug: None,
                cross_client_safe: None,
                force: Some(true),
                author_cc: "CC-Stealth".to_string(),
                source_incident_id: Some(incident_id.to_string()),
            },
        )
        .await;
        assert_eq!(add_result.is_error, Some(false));
        let text = extract_text(&add_result);
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        let knowledge_id = v.get("id").and_then(|x| x.as_str()).unwrap().to_string();

        // Sanity: knowledge is currently linked to the incident.
        assert!(
            no_ws(&text).contains(&format!("\"source_incident_id\":\"{incident_id}\"")),
            "pre-delete: knowledge should be linked to incident"
        );

        // Delete the incident. Knowledge row must survive.
        sqlx::query("DELETE FROM incidents WHERE id = $1")
            .bind(incident_id)
            .execute(&pool)
            .await
            .unwrap();

        // Knowledge row still exists, but its source_incident_id is NULL.
        let knowledge_uuid = uuid::Uuid::parse_str(&knowledge_id).unwrap();
        let fetched = ops_brain::repo::knowledge_repo::get_knowledge(&pool, knowledge_uuid)
            .await
            .unwrap()
            .expect("knowledge row must survive incident deletion");
        assert_eq!(
            fetched.source_incident_id, None,
            "FK cascade must be SET NULL, not CASCADE — incident cleanup should never delete learned lessons"
        );
        assert_eq!(
            fetched.author_cc.as_deref(),
            Some("CC-Stealth"),
            "author_cc must survive cascade unchanged"
        );

        // Cleanup
        sqlx::query("DELETE FROM knowledge WHERE id = $1")
            .bind(knowledge_uuid)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Incident Repo =====

mod incident_tests {
    use super::*;

    #[tokio::test]
    async fn create_and_get_incident() {
        let pool = pool().await;

        let incident = ops_brain::repo::incident_repo::create_incident(
            &pool,
            "Test Server Down",
            "high",
            None,
            Some("Cannot SSH"),
            Some("Created by test"),
            false,
        )
        .await
        .unwrap();

        assert_eq!(incident.title, "Test Server Down");
        assert_eq!(incident.status, "open");
        assert_eq!(incident.severity, "high");
        assert!(incident.resolved_at.is_none());
        assert!(incident.time_to_resolve_minutes.is_none());

        let fetched = ops_brain::repo::incident_repo::get_incident(&pool, incident.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.id, incident.id);

        // Cleanup
        sqlx::query("DELETE FROM incidents WHERE id = $1")
            .bind(incident.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn resolve_incident_sets_ttr() {
        let pool = pool().await;

        let incident = ops_brain::repo::incident_repo::create_incident(
            &pool,
            "TTR Test Incident",
            "medium",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        assert!(incident.resolved_at.is_none());

        let resolved = ops_brain::repo::incident_repo::update_incident(
            &pool,
            incident.id,
            None,
            Some("resolved"),
            None,
            None,
            Some("Disk was full"),
            Some("Cleared old logs"),
            None,
            None,
            None, // cross_client_safe
        )
        .await
        .unwrap();

        assert_eq!(resolved.status, "resolved");
        assert!(resolved.resolved_at.is_some());
        assert!(resolved.time_to_resolve_minutes.is_some());
        assert_eq!(resolved.root_cause.as_deref(), Some("Disk was full"));
        assert_eq!(resolved.resolution.as_deref(), Some("Cleared old logs"));

        // Cleanup
        sqlx::query("DELETE FROM incidents WHERE id = $1")
            .bind(incident.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn list_incidents_with_filters() {
        let pool = pool().await;

        let i1 = ops_brain::repo::incident_repo::create_incident(
            &pool,
            "Filter Test Open",
            "high",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        let i2 = ops_brain::repo::incident_repo::create_incident(
            &pool,
            "Filter Test Critical",
            "critical",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        // List by severity
        let critical = ops_brain::repo::incident_repo::list_incidents(
            &pool,
            None,
            None,
            Some("critical"),
            100,
        )
        .await
        .unwrap();
        assert!(critical.iter().any(|i| i.id == i2.id));

        // List by status
        let open =
            ops_brain::repo::incident_repo::list_incidents(&pool, None, Some("open"), None, 100)
                .await
                .unwrap();
        assert!(open.iter().any(|i| i.id == i1.id));
        assert!(open.iter().any(|i| i.id == i2.id));

        // Cleanup
        sqlx::query("DELETE FROM incidents WHERE id = ANY($1)")
            .bind([i1.id, i2.id])
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Incident Similarity (raid #3) =====

mod incident_similarity_tests {
    use super::*;
    use ops_brain::repo::embedding_repo;
    use ops_brain::repo::incident_repo;

    fn build_brain(pool: PgPool) -> ops_brain::tools::OpsBrain {
        ops_brain::tools::OpsBrain::new(pool, vec![], None, None)
    }

    fn extract_text(result: &rmcp::model::CallToolResult) -> String {
        result
            .content
            .first()
            .expect("result has at least one content item")
            .as_text()
            .expect("content is text")
            .text
            .clone()
    }

    /// Build a 768-dim test embedding with the given leading components,
    /// rest zero. Cosine distance between two such vectors is determined
    /// entirely by their non-zero components, so we can construct
    /// deterministic "near" and "far" pairs without a real embedding service.
    fn test_embedding(values: &[f32]) -> Vec<f32> {
        let mut v = vec![0.0_f32; 768];
        for (i, x) in values.iter().enumerate() {
            v[i] = *x;
        }
        v
    }

    async fn cleanup_incidents(pool: &PgPool, ids: &[uuid::Uuid]) {
        sqlx::query("DELETE FROM incidents WHERE id = ANY($1)")
            .bind(ids.to_vec())
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn find_similar_open_incidents_returns_close_matches() {
        let pool = pool().await;

        // Two near-identical embeddings on the same dominant axis
        let emb_existing = test_embedding(&[1.0, 0.0]);
        let emb_query = test_embedding(&[0.99, 0.01]);

        let existing = incident_repo::create_incident(
            &pool,
            "Database connection pool exhausted (similarity test)",
            "high",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        embedding_repo::store_incident_embedding(&pool, existing.id, &emb_existing)
            .await
            .unwrap();

        // Probe with a different (synthetic) id — simulates the new incident
        // querying against existing ones, excluding itself.
        let probe_id = uuid::Uuid::now_v7();
        let results =
            embedding_repo::find_similar_open_incidents(&pool, &emb_query, probe_id, 0.30, 5)
                .await
                .unwrap();

        assert!(
            results.iter().any(|r| r.id == existing.id),
            "expected existing incident to surface as similar; got {} results",
            results.len()
        );

        cleanup_incidents(&pool, &[existing.id]).await;
    }

    #[tokio::test]
    async fn find_similar_open_incidents_self_excludes() {
        let pool = pool().await;
        let emb = test_embedding(&[1.0, 0.0]);

        let me = incident_repo::create_incident(
            &pool,
            "Self-exclusion test incident",
            "low",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        embedding_repo::store_incident_embedding(&pool, me.id, &emb)
            .await
            .unwrap();

        // Query with my own embedding, exclude my own id
        let results = embedding_repo::find_similar_open_incidents(&pool, &emb, me.id, 0.30, 10)
            .await
            .unwrap();

        assert!(
            !results.iter().any(|r| r.id == me.id),
            "self-exclusion failed: an incident must not appear in its own similarity results"
        );

        cleanup_incidents(&pool, &[me.id]).await;
    }

    #[tokio::test]
    async fn find_similar_open_incidents_excludes_resolved() {
        let pool = pool().await;
        let emb = test_embedding(&[1.0, 0.0]);

        let to_be_resolved = incident_repo::create_incident(
            &pool,
            "Old fixed disk-full alert",
            "low",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        embedding_repo::store_incident_embedding(&pool, to_be_resolved.id, &emb)
            .await
            .unwrap();

        // Mark it resolved
        incident_repo::update_incident(
            &pool,
            to_be_resolved.id,
            None,
            Some("resolved"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let probe_id = uuid::Uuid::now_v7();
        let results = embedding_repo::find_similar_open_incidents(&pool, &emb, probe_id, 0.30, 10)
            .await
            .unwrap();

        assert!(
            !results.iter().any(|r| r.id == to_be_resolved.id),
            "resolved incidents must not surface in open-incidents similarity search"
        );

        cleanup_incidents(&pool, &[to_be_resolved.id]).await;
    }

    #[tokio::test]
    async fn find_similar_open_incidents_respects_distance_threshold() {
        let pool = pool().await;

        // Orthogonal vectors → cosine distance ≈ 1.0, far above 0.30
        let emb_x = test_embedding(&[1.0, 0.0]);
        let emb_y = test_embedding(&[0.0, 1.0]);

        let far_one = incident_repo::create_incident(
            &pool,
            "Unrelated billing question",
            "low",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        embedding_repo::store_incident_embedding(&pool, far_one.id, &emb_y)
            .await
            .unwrap();

        let probe_id = uuid::Uuid::now_v7();
        let results =
            embedding_repo::find_similar_open_incidents(&pool, &emb_x, probe_id, 0.30, 10)
                .await
                .unwrap();

        assert!(
            !results.iter().any(|r| r.id == far_one.id),
            "orthogonal embedding (distance ≈ 1.0) must not pass threshold 0.30"
        );

        cleanup_incidents(&pool, &[far_one.id]).await;
    }

    #[tokio::test]
    async fn nearest_open_incident_distance_returns_closest_even_over_threshold() {
        // Telemetry path: when find_similar_open_incidents returns empty
        // because nothing is below the 0.30 cutoff, nearest_open_incident_distance
        // must still return the closest distance. Regression-proofs the
        // miss-side telemetry for threshold retuning.
        //
        // The test database is shared across parallel tests, so we use
        // high-dimension unique axes (100 and 101) that no other test touches
        // — every other open incident in the db has embeddings along axes 0/1,
        // making them all cosine-distance ≈ 1.0 from our probe.
        let pool = pool().await;

        let mut emb_existing = vec![0.0_f32; 768];
        emb_existing[100] = 1.0;
        let mut emb_query = vec![0.0_f32; 768];
        emb_query[101] = 1.0;

        let existing = incident_repo::create_incident(
            &pool,
            "Unrelated billing question (miss telemetry)",
            "low",
            None,
            None,
            None,
            false,
        )
        .await
        .unwrap();
        embedding_repo::store_incident_embedding(&pool, existing.id, &emb_existing)
            .await
            .unwrap();

        let probe_id = uuid::Uuid::now_v7();

        // Confirm the threshold filter would return nothing at 0.30
        let filtered =
            embedding_repo::find_similar_open_incidents(&pool, &emb_query, probe_id, 0.30, 5)
                .await
                .unwrap();
        assert!(
            filtered.is_empty(),
            "setup precondition: orthogonal vectors (axis 100 vs 101) must not pass 0.30 \
             threshold, but got {} matches",
            filtered.len()
        );

        // But the nearest-distance telemetry helper must still see something
        let nearest = embedding_repo::nearest_open_incident_distance(&pool, &emb_query, probe_id)
            .await
            .unwrap();
        assert!(
            nearest.is_some(),
            "nearest_open_incident_distance must return Some(d) when any open incident with \
             an embedding exists, even when d is above the similarity threshold"
        );
        assert!(
            nearest.unwrap() > 0.30,
            "nearest distance for orthogonal vectors should be > 0.30 (got {:?})",
            nearest
        );

        cleanup_incidents(&pool, &[existing.id]).await;
    }

    #[tokio::test]
    async fn handler_create_incident_response_includes_similar_field() {
        // build_brain passes None for embedding_client, so the similarity
        // logic is skipped — but the response shape MUST still include
        // `_similar_incidents` (as an empty array) so callers can rely on
        // the field always being present. Guards against future regressions
        // of the response contract.
        let pool = pool().await;
        let brain = build_brain(pool.clone());

        let result = ops_brain::tools::incidents::handle_create_incident(
            &brain,
            ops_brain::tools::incidents::CreateIncidentParams {
                title: format!("Shape contract {}", uuid::Uuid::now_v7()),
                severity: Some("low".to_string()),
                client_slug: None,
                symptoms: None,
                notes: None,
                server_slugs: None,
                service_slugs: None,
                cross_client_safe: None,
                acknowledge_cross_client: None,
            },
        )
        .await;

        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        let parsed: serde_json::Value =
            serde_json::from_str(&text).expect("response is valid JSON");

        assert!(
            parsed.get("incident").is_some(),
            "response must wrap the new incident under `incident`"
        );
        assert!(
            parsed.get("_similar_incidents").is_some(),
            "response must always include `_similar_incidents` (empty when no embedding client)"
        );
        assert!(
            parsed["_similar_incidents"].is_array(),
            "_similar_incidents must be an array"
        );
        assert_eq!(
            parsed["_similar_incidents"].as_array().unwrap().len(),
            0,
            "no embedding client + clean DB = empty similar list"
        );
        assert!(
            parsed.get("_cross_client_withheld").is_none(),
            "_cross_client_withheld should be absent when nothing was withheld"
        );

        // Cleanup
        let id_str = parsed["incident"]["id"].as_str().unwrap();
        let id = uuid::Uuid::parse_str(id_str).unwrap();
        cleanup_incidents(&pool, &[id]).await;
    }
}

// ===== Session & Handoff Repo =====

mod coordination_tests {
    use super::*;

    #[tokio::test]
    async fn handoff_lifecycle() {
        let pool = pool().await;

        let handoff = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            None,
            "dev-laptop",
            Some("prod-server"),
            "high",
            "action",
            "Continue DNS migration",
            "Need to update remaining A records",
            None,
        )
        .await
        .unwrap();

        assert_eq!(handoff.status, "pending");
        assert_eq!(handoff.category, "action");
        assert_eq!(handoff.from_machine, "dev-laptop");
        assert_eq!(handoff.to_machine.as_deref(), Some("prod-server"));

        // Accept
        let accepted =
            ops_brain::repo::handoff_repo::update_handoff_status(&pool, handoff.id, "accepted")
                .await
                .unwrap();
        assert_eq!(accepted.status, "accepted");

        // Complete
        let completed =
            ops_brain::repo::handoff_repo::update_handoff_status(&pool, handoff.id, "completed")
                .await
                .unwrap();
        assert_eq!(completed.status, "completed");

        // List by status
        let pending = ops_brain::repo::handoff_repo::list_handoffs(
            &pool,
            Some("pending"),
            None,
            None,
            None,
            false,
            10,
        )
        .await
        .unwrap();
        assert!(!pending.iter().any(|h| h.id == handoff.id));

        // Cleanup
        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(handoff.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn handoff_category_default_filters_notify() {
        let pool = pool().await;

        let action = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            None,
            "dev-laptop",
            Some("category-test-host"),
            "normal",
            "action",
            "Action item",
            "Body",
            None,
        )
        .await
        .unwrap();

        let notify = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            None,
            "dev-laptop",
            Some("category-test-host"),
            "low",
            "notify",
            "FYI item",
            "Body",
            None,
        )
        .await
        .unwrap();

        // Default (include_notify=false): only action surfaces.
        let default_list = ops_brain::repo::handoff_repo::list_handoffs(
            &pool,
            Some("pending"),
            Some("category-test-host"),
            None,
            None,
            false,
            50,
        )
        .await
        .unwrap();
        assert!(default_list.iter().any(|h| h.id == action.id));
        assert!(!default_list.iter().any(|h| h.id == notify.id));

        // include_notify=true: both surface.
        let combined = ops_brain::repo::handoff_repo::list_handoffs(
            &pool,
            Some("pending"),
            Some("category-test-host"),
            None,
            None,
            true,
            50,
        )
        .await
        .unwrap();
        assert!(combined.iter().any(|h| h.id == action.id));
        assert!(combined.iter().any(|h| h.id == notify.id));

        // Explicit category=notify: only notify.
        let notify_only = ops_brain::repo::handoff_repo::list_handoffs(
            &pool,
            Some("pending"),
            Some("category-test-host"),
            None,
            Some("notify"),
            false,
            50,
        )
        .await
        .unwrap();
        assert!(notify_only.iter().any(|h| h.id == notify.id));
        assert!(!notify_only.iter().any(|h| h.id == action.id));

        // Cleanup
        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(vec![action.id, notify.id])
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Audit Log Repo =====

mod audit_log_tests {
    use super::*;

    #[tokio::test]
    async fn log_cross_client_access() {
        let pool = pool().await;

        // Create real clients for FK constraints
        let slug_req = format!("audit-req-{}", Uuid::now_v7());
        let slug_own = format!("audit-own-{}", Uuid::now_v7());

        let req_client = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Requesting Client",
            &slug_req,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let own_client = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Owning Client",
            &slug_own,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let entity_id = Uuid::now_v7();

        // Should not error
        ops_brain::repo::audit_log_repo::log_access(
            &pool,
            "search_runbooks",
            Some(req_client.id),
            "runbook",
            entity_id,
            Some(own_client.id),
            "withheld",
        )
        .await;

        // Verify it was written
        let row = sqlx::query_as::<_, (String, String)>(
            "SELECT tool_name, action FROM audit_log WHERE entity_id = $1",
        )
        .bind(entity_id)
        .fetch_optional(&pool)
        .await
        .unwrap();

        let (tool, action) = row.unwrap();
        assert_eq!(tool, "search_runbooks");
        assert_eq!(action, "withheld");

        // Cleanup
        sqlx::query("DELETE FROM audit_log WHERE entity_id = $1")
            .bind(entity_id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM clients WHERE id = ANY($1)")
            .bind([req_client.id, own_client.id])
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Briefing Repo =====

mod briefing_tests {
    use super::*;

    #[tokio::test]
    async fn insert_briefing() {
        let pool = pool().await;

        let briefing = ops_brain::repo::briefing_repo::insert_briefing(
            &pool,
            "daily",
            None,
            "# Daily Briefing\n\nAll systems operational.",
        )
        .await
        .unwrap();

        assert_eq!(briefing.briefing_type, "daily");
        assert!(briefing.client_id.is_none());
        assert_eq!(
            briefing.content,
            "# Daily Briefing\n\nAll systems operational."
        );

        // Cleanup
        sqlx::query("DELETE FROM briefings WHERE id = $1")
            .bind(briefing.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Delete Tools =====

mod delete_tests {
    use super::*;

    #[tokio::test]
    async fn delete_server_basic() {
        let pool = pool().await;
        let client_slug = format!("del-srv-client-{}", Uuid::now_v7());
        let site_slug = format!("del-srv-site-{}", Uuid::now_v7());
        let server_slug = format!("del-srv-{}", Uuid::now_v7());

        let client = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Delete Test Client",
            &client_slug,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let site = ops_brain::repo::site_repo::upsert_site(
            &pool,
            client.id,
            "Delete Test Site",
            &site_slug,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let server = ops_brain::repo::server_repo::upsert_server(
            &pool,
            site.id,
            "DEL-TEST-SRV",
            &server_slug,
            Some("Windows Server 2022"),
            &[],
            None,
            &[],
            None,
            None,
            None,
            None,
            false,
            None,
            "active",
            None,
        )
        .await
        .unwrap();

        // Check references (should be empty)
        let refs = ops_brain::repo::server_repo::count_server_references(&pool, server.id)
            .await
            .unwrap();
        assert!(refs.is_empty());

        // Delete
        let deleted = ops_brain::repo::server_repo::delete_server(&pool, server.id)
            .await
            .unwrap();
        assert!(deleted);

        // Verify gone
        let found = ops_brain::repo::server_repo::get_server_by_slug(&pool, &server_slug)
            .await
            .unwrap();
        assert!(found.is_none());

        // Cleanup
        sqlx::query("DELETE FROM sites WHERE id = $1")
            .bind(site.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(client.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_server_with_service_link() {
        let pool = pool().await;
        let client_slug = format!("del-link-client-{}", Uuid::now_v7());
        let site_slug = format!("del-link-site-{}", Uuid::now_v7());
        let server_slug = format!("del-link-srv-{}", Uuid::now_v7());
        let service_slug = format!("del-link-svc-{}", Uuid::now_v7());

        let client = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Link Test Client",
            &client_slug,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let site = ops_brain::repo::site_repo::upsert_site(
            &pool,
            client.id,
            "Link Test Site",
            &site_slug,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let server = ops_brain::repo::server_repo::upsert_server(
            &pool,
            site.id,
            "DEL-LINK-SRV",
            &server_slug,
            None,
            &[],
            None,
            &[],
            None,
            None,
            None,
            None,
            false,
            None,
            "active",
            None,
        )
        .await
        .unwrap();

        let service = ops_brain::repo::service_repo::upsert_service(
            &pool,
            "Test Service for Delete",
            &service_slug,
            Some("test"),
            None,
            "low",
            None,
        )
        .await
        .unwrap();

        // Link server to service
        ops_brain::repo::service_repo::link_server_service(
            &pool, server.id, service.id, None, None,
        )
        .await
        .unwrap();

        // Check references — should show 1 linked service
        let refs = ops_brain::repo::server_repo::count_server_references(&pool, server.id)
            .await
            .unwrap();
        assert!(refs
            .iter()
            .any(|(name, count)| name == "linked services" && *count == 1));

        // Delete — CASCADE should remove server_services link
        let deleted = ops_brain::repo::server_repo::delete_server(&pool, server.id)
            .await
            .unwrap();
        assert!(deleted);

        // Service should still exist
        let svc = ops_brain::repo::service_repo::get_service_by_slug(&pool, &service_slug)
            .await
            .unwrap();
        assert!(svc.is_some());

        // Cleanup
        sqlx::query("DELETE FROM services WHERE id = $1")
            .bind(service.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM sites WHERE id = $1")
            .bind(site.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(client.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn delete_service_basic() {
        let pool = pool().await;
        let slug = format!("del-svc-{}", Uuid::now_v7());

        let service = ops_brain::repo::service_repo::upsert_service(
            &pool,
            "Delete Test Service",
            &slug,
            Some("test"),
            None,
            "low",
            None,
        )
        .await
        .unwrap();

        let refs = ops_brain::repo::service_repo::count_service_references(&pool, service.id)
            .await
            .unwrap();
        assert!(refs.is_empty());

        let deleted = ops_brain::repo::service_repo::delete_service(&pool, service.id)
            .await
            .unwrap();
        assert!(deleted);

        let found = ops_brain::repo::service_repo::get_service_by_slug(&pool, &slug)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_vendor_basic() {
        let pool = pool().await;
        let name = format!("Delete Test Vendor {}", Uuid::now_v7());

        let vendor = ops_brain::repo::vendor_repo::upsert_vendor(
            &pool,
            &name,
            Some("test"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let refs = ops_brain::repo::vendor_repo::count_vendor_references(&pool, vendor.id)
            .await
            .unwrap();
        assert!(refs.is_empty());

        let deleted = ops_brain::repo::vendor_repo::delete_vendor(&pool, vendor.id)
            .await
            .unwrap();
        assert!(deleted);

        let found = ops_brain::repo::vendor_repo::get_vendor_by_name(&pool, &name)
            .await
            .unwrap();
        assert!(found.is_none());
    }

    #[tokio::test]
    async fn delete_nonexistent_returns_false() {
        let pool = pool().await;
        let fake_id = Uuid::now_v7();

        let result = ops_brain::repo::server_repo::delete_server(&pool, fake_id)
            .await
            .unwrap();
        assert!(!result);

        let result = ops_brain::repo::service_repo::delete_service(&pool, fake_id)
            .await
            .unwrap();
        assert!(!result);

        let result = ops_brain::repo::vendor_repo::delete_vendor(&pool, fake_id)
            .await
            .unwrap();
        assert!(!result);
    }
}

// ===== Fuzzy Slug Suggestion Tests =====

mod suggest_tests {
    use super::*;

    #[tokio::test]
    async fn suggest_similar_server_slugs() {
        let pool = pool().await;
        let base_slug = format!("fuzzy-srv-{}", &Uuid::now_v7().to_string()[..8]);

        // Create a test site + client first
        let client = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Fuzzy Test Client",
            &format!("fuzzy-client-{}", &Uuid::now_v7().to_string()[..8]),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        let site = ops_brain::repo::site_repo::upsert_site(
            &pool,
            client.id,
            "Fuzzy Test Site",
            &format!("fuzzy-site-{}", &Uuid::now_v7().to_string()[..8]),
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // Create a server with a known slug
        let server = ops_brain::repo::server_repo::upsert_server(
            &pool,
            site.id,
            "fuzzy-test-host",
            &base_slug,
            None,
            &[],
            None,
            &[],
            None,
            None,
            None,
            None,
            false,
            None,
            "active",
            None,
        )
        .await
        .unwrap();

        // Typo slug should return suggestions
        let typo = format!("{}x", &base_slug);
        let suggestions =
            ops_brain::repo::suggest_repo::suggest_similar_slugs(&pool, "servers", &typo).await;
        assert!(
            suggestions.contains(&base_slug),
            "Expected '{}' in suggestions {:?}",
            base_slug,
            suggestions
        );

        // Exact slug should NOT appear when querying with something totally unrelated
        let suggestions = ops_brain::repo::suggest_repo::suggest_similar_slugs(
            &pool,
            "servers",
            "zzz-no-match-zzz",
        )
        .await;
        assert!(
            !suggestions.contains(&base_slug),
            "Should not suggest '{}' for unrelated query",
            base_slug
        );

        // Substring match should work
        let partial = &base_slug[..base_slug.len() - 2];
        let suggestions =
            ops_brain::repo::suggest_repo::suggest_similar_slugs(&pool, "servers", partial).await;
        assert!(
            suggestions.contains(&base_slug),
            "Substring '{}' should match '{}', got {:?}",
            partial,
            base_slug,
            suggestions
        );

        // Cleanup
        sqlx::query("DELETE FROM servers WHERE id = $1")
            .bind(server.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM sites WHERE id = $1")
            .bind(site.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(client.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn suggest_similar_vendor_names() {
        let pool = pool().await;
        let vendor_name = format!("FuzzyVendor-{}", &Uuid::now_v7().to_string()[..8]);

        let vendor = ops_brain::repo::vendor_repo::upsert_vendor(
            &pool,
            &vendor_name,
            Some("test"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // Typo should return suggestion
        let typo = format!("{}x", &vendor_name);
        let suggestions =
            ops_brain::repo::suggest_repo::suggest_similar_vendor_names(&pool, &typo).await;
        assert!(
            suggestions.contains(&vendor_name),
            "Expected '{}' in suggestions {:?}",
            vendor_name,
            suggestions
        );

        // Cleanup
        sqlx::query("DELETE FROM vendors WHERE id = $1")
            .bind(vendor.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn suggest_returns_empty_for_invalid_table() {
        let pool = pool().await;
        let suggestions =
            ops_brain::repo::suggest_repo::suggest_similar_slugs(&pool, "nonexistent", "test")
                .await;
        assert!(suggestions.is_empty());
    }
}

// ===== Runbook Execution Repo =====

// ===== Vendor Dedup Tests =====

mod vendor_dedup_tests {
    use super::*;

    #[tokio::test]
    async fn upsert_vendor_same_name_updates_instead_of_duplicating() {
        let pool = pool().await;
        let name = format!("TestVendor-{}", Uuid::now_v7());

        // First insert
        let v1 = ops_brain::repo::vendor_repo::upsert_vendor(
            &pool,
            &name,
            Some("isp"),
            Some("ACCT-001"),
            Some("555-0001"),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(v1.name, name);
        assert_eq!(v1.category.as_deref(), Some("isp"));
        assert_eq!(v1.account_number.as_deref(), Some("ACCT-001"));

        // Second upsert with same name — should update, not duplicate
        let v2 = ops_brain::repo::vendor_repo::upsert_vendor(
            &pool,
            &name,
            Some("hardware"),
            None, // account_number not provided — should preserve ACCT-001
            Some("555-0002"),
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // Same row (same ID)
        assert_eq!(v2.id, v1.id);
        // Category updated
        assert_eq!(v2.category.as_deref(), Some("hardware"));
        // account_number preserved (COALESCE)
        assert_eq!(v2.account_number.as_deref(), Some("ACCT-001"));
        // support_phone updated
        assert_eq!(v2.support_phone.as_deref(), Some("555-0002"));

        // Verify only one row exists
        let found = ops_brain::repo::vendor_repo::get_vendor_by_name(&pool, &name)
            .await
            .unwrap();
        assert!(found.is_some());

        // Cleanup
        sqlx::query("DELETE FROM vendors WHERE id = $1")
            .bind(v1.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn upsert_vendor_case_insensitive_dedup() {
        let pool = pool().await;
        let base = format!("CaseVendor-{}", Uuid::now_v7());
        let upper = base.to_uppercase();

        let v1 = ops_brain::repo::vendor_repo::upsert_vendor(
            &pool,
            &base,
            Some("software"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        // Upsert with different casing — should hit ON CONFLICT
        let v2 = ops_brain::repo::vendor_repo::upsert_vendor(
            &pool,
            &upper,
            Some("cloud"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(v2.id, v1.id);
        assert_eq!(v2.category.as_deref(), Some("cloud"));

        // Cleanup
        sqlx::query("DELETE FROM vendors WHERE id = $1")
            .bind(v1.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Server Partial Update Tests =====

mod server_partial_update_tests {
    use super::*;

    async fn create_test_server(
        pool: &sqlx::PgPool,
    ) -> (uuid::Uuid, uuid::Uuid, String, String, String) {
        let client_slug = format!("test-pu-client-{}", Uuid::now_v7());
        let site_slug = format!("test-pu-site-{}", Uuid::now_v7());
        let server_slug = format!("test-pu-srv-{}", Uuid::now_v7());

        let client = ops_brain::repo::client_repo::upsert_client(
            pool,
            "PU Test Client",
            &client_slug,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let site = ops_brain::repo::site_repo::upsert_site(
            pool,
            client.id,
            "PU Test Site",
            &site_slug,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let _server = ops_brain::repo::server_repo::upsert_server(
            pool,
            site.id,
            "PU-TEST-SRV",
            &server_slug,
            Some("Windows Server 2022"),
            &["10.0.0.50".to_string()],
            Some("pu-test"),
            &["file-server".to_string(), "dns".to_string()],
            Some("Dell R740"),
            Some("Xeon E-2288G"),
            Some(64),
            Some("2x 1TB SSD RAID1"),
            false,
            None,
            "active",
            Some("Original notes"),
        )
        .await
        .unwrap();

        (client.id, site.id, client_slug, site_slug, server_slug)
    }

    async fn cleanup(
        pool: &sqlx::PgPool,
        server_slug: &str,
        site_id: uuid::Uuid,
        client_id: uuid::Uuid,
    ) {
        sqlx::query("DELETE FROM servers WHERE slug = $1")
            .bind(server_slug)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM sites WHERE id = $1")
            .bind(site_id)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(client_id)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn partial_update_preserves_unprovided_fields() {
        let pool = pool().await;
        let (client_id, site_id, _client_slug, _site_slug, server_slug) =
            create_test_server(&pool).await;

        // Partial update: only change OS, leave everything else
        let updated = ops_brain::repo::server_repo::update_server_partial(
            &pool,
            &server_slug,
            None,                        // site_id: preserve
            None,                        // hostname: preserve
            Some("Windows Server 2025"), // os: update
            None,                        // ip_addresses: preserve
            None,                        // ssh_alias: preserve
            None,                        // roles: preserve
            None,                        // hardware: preserve
            None,                        // cpu: preserve
            None,                        // ram_gb: preserve
            None,                        // storage_summary: preserve
            None,                        // is_virtual: preserve
            None,                        // hypervisor_id: preserve
            None,                        // status: preserve
            None,                        // notes: preserve
        )
        .await
        .unwrap();

        // OS should be updated
        assert_eq!(updated.os.as_deref(), Some("Windows Server 2025"));
        // Everything else should be preserved
        assert_eq!(updated.hostname, "PU-TEST-SRV");
        assert_eq!(updated.ip_addresses, vec!["10.0.0.50"]);
        assert_eq!(updated.roles, vec!["file-server", "dns"]);
        assert_eq!(updated.hardware.as_deref(), Some("Dell R740"));
        assert_eq!(updated.cpu.as_deref(), Some("Xeon E-2288G"));
        assert_eq!(updated.ram_gb, Some(64));
        assert_eq!(updated.storage_summary.as_deref(), Some("2x 1TB SSD RAID1"));
        assert!(!updated.is_virtual);
        assert_eq!(updated.status, "active");
        assert_eq!(updated.notes.as_deref(), Some("Original notes"));
        assert_eq!(updated.ssh_alias.as_deref(), Some("pu-test"));

        cleanup(&pool, &server_slug, site_id, client_id).await;
    }

    #[tokio::test]
    async fn partial_update_can_change_multiple_fields() {
        let pool = pool().await;
        let (client_id, site_id, _client_slug, _site_slug, server_slug) =
            create_test_server(&pool).await;

        let updated = ops_brain::repo::server_repo::update_server_partial(
            &pool,
            &server_slug,
            None,
            Some("RENAMED-SRV"),
            None,
            Some(&["10.0.0.51".to_string(), "10.0.0.52".to_string()]),
            None,
            Some(&["dc".to_string()]),
            None,
            None,
            Some(128),
            None,
            Some(true),
            None,
            None,
            Some("Updated notes"),
        )
        .await
        .unwrap();

        assert_eq!(updated.hostname, "RENAMED-SRV");
        assert_eq!(updated.ip_addresses, vec!["10.0.0.51", "10.0.0.52"]);
        assert_eq!(updated.roles, vec!["dc"]);
        assert_eq!(updated.ram_gb, Some(128));
        assert!(updated.is_virtual);
        assert_eq!(updated.notes.as_deref(), Some("Updated notes"));
        // Unchanged fields preserved
        assert_eq!(updated.os.as_deref(), Some("Windows Server 2022"));
        assert_eq!(updated.hardware.as_deref(), Some("Dell R740"));
        assert_eq!(updated.cpu.as_deref(), Some("Xeon E-2288G"));

        cleanup(&pool, &server_slug, site_id, client_id).await;
    }

    #[tokio::test]
    async fn partial_update_hypervisor_explicit_set() {
        let pool = pool().await;
        let (client_id, site_id, _client_slug, _site_slug, server_slug) =
            create_test_server(&pool).await;

        // Create a hypervisor
        let hyp_slug = format!("test-hyp-{}", Uuid::now_v7());
        let hypervisor = ops_brain::repo::server_repo::upsert_server(
            &pool,
            site_id,
            "HYP-SRV",
            &hyp_slug,
            None,
            &[],
            None,
            &["hypervisor".to_string()],
            None,
            None,
            None,
            None,
            false,
            None,
            "active",
            None,
        )
        .await
        .unwrap();

        // Set hypervisor_id via partial update
        let updated = ops_brain::repo::server_repo::update_server_partial(
            &pool,
            &server_slug,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            None,
            Some(true),                // is_virtual
            Some(Some(hypervisor.id)), // hypervisor_id: explicitly set
            None,
            None,
        )
        .await
        .unwrap();

        assert!(updated.is_virtual);
        assert_eq!(updated.hypervisor_id, Some(hypervisor.id));
        // Everything else preserved
        assert_eq!(updated.os.as_deref(), Some("Windows Server 2022"));
        assert_eq!(updated.roles, vec!["file-server", "dns"]);

        // Cleanup
        sqlx::query("DELETE FROM servers WHERE slug = $1")
            .bind(&hyp_slug)
            .execute(&pool)
            .await
            .unwrap();
        cleanup(&pool, &server_slug, site_id, client_id).await;
    }
}

// ===== check_in handler =====
//
// check_in is now a stateless pending-work query (open handoffs to your
// machine, recent notify-class handoffs, open incidents in your scope). The
// CC_TEAM allowlist is unit-tested in `src/tools/cc_team.rs`; this integration
// test covers the handler's invalid-name rejection because that's the one
// branch that needs an OpsBrain to exercise the error path end-to-end.

mod check_in_tests {
    use super::*;

    fn build_brain(pool: PgPool) -> ops_brain::tools::OpsBrain {
        ops_brain::tools::OpsBrain::new(pool, vec![], None, None)
    }

    fn extract_text(result: &rmcp::model::CallToolResult) -> String {
        result
            .content
            .first()
            .expect("result has at least one content item")
            .as_text()
            .expect("content is text")
            .text
            .clone()
    }

    #[tokio::test]
    async fn handler_check_in_rejects_invalid_name() {
        let brain = build_brain(pool().await);
        let result = ops_brain::tools::cc_team::handle_check_in(
            &brain,
            ops_brain::tools::cc_team::CheckInParams {
                my_name: "CC-NotReal".to_string(),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(text.contains("Invalid CC name"));
        assert!(text.contains("CC-Cloud"), "error should list valid names");
    }

    #[tokio::test]
    async fn handler_check_in_accepts_valid_name() {
        let brain = build_brain(pool().await);
        let result = ops_brain::tools::cc_team::handle_check_in(
            &brain,
            ops_brain::tools::cc_team::CheckInParams {
                my_name: "CC-Stealth".to_string(),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        // The three things a sovereign CC needs from the bus.
        assert!(text.contains("open_handoffs_to_you"));
        assert!(text.contains("recent_notifications"));
        assert!(text.contains("open_incidents_in_your_scope"));
        // v1.5 regression guards: identity echo must NOT be in the response.
        // Local is the source of truth — the CC already knows its own name
        // and hostname; echoing them back was the last trace of the v1.4
        // "tell me who I am" framing.
        assert!(
            !text.contains("\"you\":"),
            "v1.5: `you` field must not echo CC name back — identity is local"
        );
        assert!(
            !text.contains("\"hostname\":"),
            "v1.5: `hostname` field must not echo back — local is the source of truth"
        );
    }
}
