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
        )
        .await
        .unwrap();

        assert_eq!(k.title, "Test Knowledge Entry");
        assert!(!k.cross_client_safe);
        assert_eq!(k.author.as_deref(), Some("CC-Stealth"));

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

// ===== Knowledge Provenance =====
//
// Pure-logic tests for `author` validation, `is_knowledge_stale`, and
// `knowledge_entries_to_json` live in `src/tools/knowledge.rs`. The
// integration tests above (`add_and_get_knowledge`,
// `knowledge_cross_client_safe_field`) exercise the handler-layer paths
// that need a real OpsBrain to round-trip through.

// ===== Handoffs =====

mod coordination_tests {
    use super::*;

    #[tokio::test]
    async fn handoff_lifecycle() {
        let pool = pool().await;

        let handoff = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            "dev-laptop",
            Some("prod-server"),
            "high",
            "action",
            "Continue DNS migration",
            "Need to update remaining A records",
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(handoff.status, "pending");
        assert_eq!(handoff.category, "action");
        assert_eq!(handoff.from_agent, "dev-laptop");
        assert_eq!(handoff.to_agent.as_deref(), Some("prod-server"));

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
            "dev-laptop",
            Some("category-test-host"),
            "normal",
            "action",
            "Action item",
            "Body",
            None,
            None,
        )
        .await
        .unwrap();

        let notify = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            "dev-laptop",
            Some("category-test-host"),
            "low",
            "notify",
            "FYI item",
            "Body",
            None,
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

    /// v2.0 keeps handoff routing agent-agnostic: values are stored exactly
    /// as provided after validation. Legacy CC names remain valid, but there
    /// is no hostname-to-CC normalization on new writes.
    #[tokio::test]
    async fn handoff_agent_names_are_preserved_exactly() {
        let pool = pool().await;

        let cases = [
            ("CC-Stealth", Some("CC-Cloud")),
            ("Codex-HSR", Some("Gemini-HSR")),
            ("opencode.local", None),
        ];
        let mut ids = Vec::new();
        for (from_agent, to_agent) in &cases {
            let h = ops_brain::repo::handoff_repo::create_handoff(
                &pool,
                from_agent,
                *to_agent,
                "normal",
                "action",
                "agent preservation test",
                "body",
                None,
                None,
            )
            .await
            .unwrap();
            ids.push(h.id);
        }

        for (id, (expected_from, expected_to)) in ids.iter().zip(cases.iter()) {
            let row: (String, Option<String>) =
                sqlx::query_as("SELECT from_agent, to_agent FROM handoffs WHERE id = $1")
                    .bind(id)
                    .fetch_one(&pool)
                    .await
                    .unwrap();
            assert_eq!(row.0, *expected_from, "from_agent for row {id}");
            assert_eq!(row.1.as_deref(), *expected_to, "to_agent for row {id}");
        }

        // Cleanup
        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// in_reply_to threads a child handoff to a parent and list_replies_to_me
    /// surfaces it via the parent's from_agent (not the reply's).
    #[tokio::test]
    async fn handoff_in_reply_to_threading() {
        let pool = pool().await;
        let alice = format!("alice-{}", Uuid::now_v7());
        let bob = format!("bob-{}", Uuid::now_v7());

        let parent = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            &alice,
            Some(&bob),
            "normal",
            "action",
            "Please review",
            "body",
            None,
            None,
        )
        .await
        .unwrap();

        let reply = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            &bob,
            Some(&alice),
            "normal",
            "action",
            "Re: Please review",
            "looks good",
            None,
            Some(parent.id),
        )
        .await
        .unwrap();

        assert_eq!(reply.in_reply_to, Some(parent.id));
        // Category is preserved — the reply stays `action` even though it's a reply.
        assert_eq!(reply.category, "action");

        let replies = ops_brain::repo::handoff_repo::list_replies_to_me(&pool, &alice, None, 10)
            .await
            .unwrap();
        assert!(replies.iter().any(|h| h.id == reply.id));
        // Parent author asking for *their* replies shouldn't see unrelated rows.
        assert!(replies.iter().all(|h| h.in_reply_to == Some(parent.id)));

        // Cleanup — order matters because of the FK.
        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(reply.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(parent.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// in_reply_to FK is ON DELETE SET NULL — deleting a parent leaves the
    /// reply intact with a nulled link, never cascade-orphaned.
    #[tokio::test]
    async fn handoff_reply_survives_parent_deletion() {
        let pool = pool().await;
        let from = format!("from-{}", Uuid::now_v7());

        let parent = ops_brain::repo::handoff_repo::create_handoff(
            &pool, &from, None, "normal", "action", "Parent", "body", None, None,
        )
        .await
        .unwrap();
        let reply = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            &from,
            None,
            "normal",
            "notify",
            "Reply",
            "body",
            None,
            Some(parent.id),
        )
        .await
        .unwrap();

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(parent.id)
            .execute(&pool)
            .await
            .unwrap();

        let row: (Option<Uuid>,) = sqlx::query_as("SELECT in_reply_to FROM handoffs WHERE id = $1")
            .bind(reply.id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            row.0.is_none(),
            "in_reply_to should null out on parent delete"
        );

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(reply.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// complete_handoff_with_commit stores the commit_hash and flips status
    /// to completed. Re-running with a different commit doesn't silently
    /// overwrite (guarded at the tool layer; repo layer keeps it permissive
    /// for explicit-reset paths).
    #[tokio::test]
    async fn handoff_complete_with_commit_hash() {
        let pool = pool().await;
        let from = format!("from-{}", Uuid::now_v7());

        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool, &from, None, "normal", "action", "Work", "body", None, None,
        )
        .await
        .unwrap();

        let completed = ops_brain::repo::handoff_repo::complete_handoff_with_commit(
            &pool,
            h.id,
            Some("abc1234"),
        )
        .await
        .unwrap();

        assert_eq!(completed.status, "completed");
        assert_eq!(completed.commit_hash.as_deref(), Some("abc1234"));

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(h.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// mark_merged flips completed handoffs to merged and records merge_commit + merged_at.
    #[tokio::test]
    async fn handoff_mark_merged_flips_status_and_records_commit() {
        let pool = pool().await;
        let from = format!("from-{}", Uuid::now_v7());

        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool, &from, None, "normal", "action", "Work", "body", None, None,
        )
        .await
        .unwrap();
        let _ = ops_brain::repo::handoff_repo::complete_handoff_with_commit(
            &pool,
            h.id,
            Some("feedf00d"),
        )
        .await
        .unwrap();

        let merged = ops_brain::repo::handoff_repo::mark_merged(&pool, h.id, "deadbeef0001")
            .await
            .unwrap();

        assert_eq!(merged.status, "merged");
        assert_eq!(merged.merge_commit.as_deref(), Some("deadbeef0001"));
        assert!(merged.merged_at.is_some());
        // commit_hash from the completion step is preserved.
        assert_eq!(merged.commit_hash.as_deref(), Some("feedf00d"));

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(h.id)
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
            "search_knowledge",
            Some(req_client.id),
            "knowledge",
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
        assert_eq!(tool, "search_knowledge");
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

// ===== Fuzzy Slug Suggestion Tests =====

// ===== check_in handler =====
//
// check_in is a stateless pending-work query (open action handoffs to your
// agent + recent notify-class handoffs). Agent slug validation is
// unit-tested in `src/validation.rs`; this integration test covers the
// handler's invalid-name rejection because that's the one branch that
// needs an OpsBrain to exercise the error path end-to-end.

mod check_in_tests {
    use super::*;

    fn build_brain(pool: PgPool) -> ops_brain::tools::OpsBrain {
        ops_brain::tools::OpsBrain::new(pool, None, None)
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
        let result = ops_brain::tools::check_in::handle_check_in(
            &brain,
            ops_brain::tools::check_in::CheckInParams {
                agent_name: "bad agent".to_string(),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(text.contains("invalid characters"));
    }

    #[tokio::test]
    async fn handler_check_in_accepts_valid_name() {
        let brain = build_brain(pool().await);
        let result = ops_brain::tools::check_in::handle_check_in(
            &brain,
            ops_brain::tools::check_in::CheckInParams {
                agent_name: "CC-Stealth".to_string(),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        // The three things an agent needs from the bus.
        assert!(text.contains("open_handoffs_to_you"));
        assert!(text.contains("recent_notifications"));
        // v1.5 regression guards: identity echo must NOT be in the response.
        // Local is the source of truth — the agent already knows its own name;
        // echoing identity back was the last trace of the v1.4
        // "tell me who I am" framing.
        assert!(
            !text.contains("\"you\":"),
            "v1.5: `you` field must not echo agent name back — identity is local"
        );
        assert!(
            !text.contains("\"hostname\":"),
            "v1.5: `hostname` field must not echo back — local is the source of truth"
        );
    }

    #[tokio::test]
    async fn handler_check_in_includes_accepted_action_handoffs() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let agent = format!("Codex-{}", Uuid::now_v7().simple());

        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            "CC-Stealth",
            Some(&agent),
            "normal",
            "action",
            "accepted visibility smoke",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        let _ = ops_brain::repo::handoff_repo::update_handoff_status(&pool, h.id, "accepted")
            .await
            .unwrap();

        let result = ops_brain::tools::check_in::handle_check_in(
            &brain,
            ops_brain::tools::check_in::CheckInParams { agent_name: agent },
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        assert!(text.contains("accepted visibility smoke"));
        assert!(text.contains("\"accepted_count\": 1"));

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(h.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// Handler-layer tests for the v3.1.0 safety guards on threading + commit
// linkage. Repo-level happy paths are covered in `coordination_tests` above;
// these pin the tool-layer behavior (invalid input, not-found, conflict-refuse,
// idempotency) the locked design promises.
mod coordination_handler_tests {
    use super::*;
    use ops_brain::tools::coordination::{
        handle_create_handoff, handle_mark_merged, CreateHandoffParams, MarkMergedParams,
    };

    fn build_brain(pool: PgPool) -> ops_brain::tools::OpsBrain {
        ops_brain::tools::OpsBrain::new(pool, None, None)
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
    async fn create_handoff_rejects_malformed_in_reply_to() {
        let brain = build_brain(pool().await);
        let result = handle_create_handoff(
            &brain,
            CreateHandoffParams {
                from_agent: "CC-Stealth".to_string(),
                to_agent: None,
                priority: None,
                category: None,
                title: "smoke".to_string(),
                body: "body".to_string(),
                context: None,
                in_reply_to: Some("not-a-uuid".to_string()),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(
            text.contains("Invalid in_reply_to UUID"),
            "expected in_reply_to validation error, got: {text}"
        );
    }

    #[tokio::test]
    async fn mark_merged_returns_not_found_for_missing_handoff() {
        let brain = build_brain(pool().await);
        let missing = Uuid::now_v7();
        let result = handle_mark_merged(
            &brain,
            MarkMergedParams {
                handoff_id: missing.to_string(),
                merge_commit: "abc1234".to_string(),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(
            text.contains("not found") || text.contains("Handoff"),
            "expected not-found surface, got: {text}"
        );
    }

    #[tokio::test]
    async fn mark_merged_is_idempotent_on_same_commit() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let from = format!("from-{}", Uuid::now_v7());

        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            &from,
            None,
            "normal",
            "action",
            "idempotency-smoke",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        let _ = ops_brain::repo::handoff_repo::complete_handoff_with_commit(
            &pool,
            h.id,
            Some("work-abc"),
        )
        .await
        .unwrap();

        let params = || MarkMergedParams {
            handoff_id: h.id.to_string(),
            merge_commit: "merge-abc".to_string(),
        };

        let first = handle_mark_merged(&brain, params()).await;
        assert_eq!(
            first.is_error,
            Some(false),
            "first mark_merged should succeed"
        );

        let second = handle_mark_merged(&brain, params()).await;
        assert_eq!(
            second.is_error,
            Some(false),
            "second mark_merged with same commit should be a no-op success"
        );

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(h.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mark_merged_refuses_to_overwrite_with_different_commit() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let from = format!("from-{}", Uuid::now_v7());

        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            &from,
            None,
            "normal",
            "action",
            "conflict-refuse-smoke",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        let _ = ops_brain::repo::handoff_repo::complete_handoff_with_commit(
            &pool,
            h.id,
            Some("work-conflict"),
        )
        .await
        .unwrap();

        let first = handle_mark_merged(
            &brain,
            MarkMergedParams {
                handoff_id: h.id.to_string(),
                merge_commit: "first-merge".to_string(),
            },
        )
        .await;
        assert_eq!(first.is_error, Some(false));

        let second = handle_mark_merged(
            &brain,
            MarkMergedParams {
                handoff_id: h.id.to_string(),
                merge_commit: "different-merge".to_string(),
            },
        )
        .await;
        assert_eq!(
            second.is_error,
            Some(true),
            "second mark_merged with different commit should refuse"
        );
        let text = extract_text(&second);
        assert!(
            text.contains("already merged") || text.contains("refusing to overwrite"),
            "expected conflict-refuse error, got: {text}"
        );

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(h.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mark_merged_rejects_uncompleted_handoff() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let from = format!("from-{}", Uuid::now_v7());

        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            &from,
            None,
            "normal",
            "action",
            "pending-merge-refuse-smoke",
            "body",
            None,
            None,
        )
        .await
        .unwrap();

        let result = handle_mark_merged(
            &brain,
            MarkMergedParams {
                handoff_id: h.id.to_string(),
                merge_commit: "merge-pending".to_string(),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(
            text.contains("must be completed"),
            "expected completed-before-merged error, got: {text}"
        );

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(h.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn mark_merged_rejects_completed_handoff_without_commit_hash() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let from = format!("from-{}", Uuid::now_v7());

        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            &from,
            None,
            "normal",
            "action",
            "missing-work-ref-smoke",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        let _ = ops_brain::repo::handoff_repo::complete_handoff_with_commit(&pool, h.id, None)
            .await
            .unwrap();

        let result = handle_mark_merged(
            &brain,
            MarkMergedParams {
                handoff_id: h.id.to_string(),
                merge_commit: "merge-no-work-ref".to_string(),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(
            text.contains("commit_hash"),
            "expected commit_hash requirement error, got: {text}"
        );

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(h.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
