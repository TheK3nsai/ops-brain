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

        let _c1 = ops_brain::repo::client_repo::upsert_client(&pool, "Original", &slug, Some("v1"))
            .await
            .unwrap();

        let c2 = ops_brain::repo::client_repo::upsert_client(&pool, "Updated", &slug, Some("v2"))
            .await
            .unwrap();

        // Same slug, should update name and notes
        assert_eq!(c2.slug, slug);
        assert_eq!(c2.name, "Updated");
        assert_eq!(c2.notes.as_deref(), Some("v2"));

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

        // Accept (atomic: only flips a pending row)
        let accepted = ops_brain::repo::handoff_repo::accept_handoff(&pool, handoff.id)
            .await
            .unwrap()
            .expect("pending handoff should accept");
        assert_eq!(accepted.status, "accepted");

        // Accepting again loses the precondition and returns None.
        let reaccept = ops_brain::repo::handoff_repo::accept_handoff(&pool, handoff.id)
            .await
            .unwrap();
        assert!(reaccept.is_none(), "second accept must not win");

        // Complete
        let completed =
            ops_brain::repo::handoff_repo::complete_handoff_with_commit(&pool, handoff.id, None)
                .await
                .unwrap()
                .expect("open handoff should complete");
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
        .unwrap()
        .expect("open handoff should complete");

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
            .unwrap()
            .expect("completed handoff should mark merged");

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
        )
        .await
        .unwrap();

        let own_client =
            ops_brain::repo::client_repo::upsert_client(&pool, "Owning Client", &slug_own, None)
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
        ops_brain::tools::OpsBrain::new(pool, None)
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
        let _ = ops_brain::repo::handoff_repo::accept_handoff(&pool, h.id)
            .await
            .unwrap()
            .expect("accept should succeed");

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
        ops_brain::tools::OpsBrain::new(pool, None)
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

mod machine_handoff_tests {
    use super::*;

    /// The pilot acceptance round-trip: file fresh → duplicate suppresses
    /// with a repeat_count bump → complete releases the key → next filing
    /// is fresh again.
    #[tokio::test]
    async fn machine_dedupe_lifecycle() {
        let pool = pool().await;
        let key = format!("test-check-{}", uuid::Uuid::now_v7());

        let first = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            "test-agent",
            "high",
            "action",
            "[auto] test check FAIL",
            "measured value vs threshold",
            None,
            Some(&key),
        )
        .await
        .unwrap();
        assert_eq!(first.origin, "machine");
        assert_eq!(first.status, "pending");
        assert_eq!(first.repeat_count, 0);
        assert_eq!(first.dedupe_key.as_deref(), Some(key.as_str()));

        // Second nightly run: suppressed into the same row, count bumped,
        // updated_at moved forward.
        let second = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            "test-agent",
            "high",
            "action",
            "[auto] test check FAIL",
            "measured value vs threshold",
            None,
            Some(&key),
        )
        .await
        .unwrap();
        assert_eq!(second.id, first.id);
        assert_eq!(second.repeat_count, 1);
        assert!(second.updated_at > first.updated_at);

        // Suppression also holds across accepted (still open).
        ops_brain::repo::handoff_repo::accept_handoff(&pool, first.id)
            .await
            .unwrap()
            .expect("accept should succeed");
        let third = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            "test-agent",
            "high",
            "action",
            "[auto] test check FAIL",
            "measured value vs threshold",
            None,
            Some(&key),
        )
        .await
        .unwrap();
        assert_eq!(third.id, first.id);
        assert_eq!(third.repeat_count, 2);

        // Completion releases the key: the same check failing later files fresh.
        ops_brain::repo::handoff_repo::complete_handoff_with_commit(&pool, first.id, None)
            .await
            .unwrap()
            .expect("complete should succeed");
        let fresh = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            "test-agent",
            "high",
            "action",
            "[auto] test check FAIL",
            "measured value vs threshold",
            None,
            Some(&key),
        )
        .await
        .unwrap();
        assert_ne!(fresh.id, first.id);
        assert_eq!(fresh.repeat_count, 0);
        assert_eq!(fresh.status, "pending");

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(vec![first.id, fresh.id])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// NULL dedupe keys never collide — every filing inserts.
    #[tokio::test]
    async fn machine_null_dedupe_keys_always_insert() {
        let pool = pool().await;

        let a = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            "test-agent",
            "normal",
            "action",
            "[auto] one-off finding",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        let b = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            "test-agent",
            "normal",
            "action",
            "[auto] one-off finding",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        assert_ne!(a.id, b.id);
        assert_eq!(a.repeat_count, 0);
        assert_eq!(b.repeat_count, 0);

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(vec![a.id, b.id])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// Dedupe scope is per recipient: the same key filed to two different
    /// agents inserts independently instead of suppressing the second
    /// filing into the first agent's handoff.
    #[tokio::test]
    async fn machine_dedupe_key_is_scoped_per_recipient() {
        let pool = pool().await;
        let key = format!("shared-check-{}", Uuid::now_v7());

        let file = |to: &'static str| {
            let pool = pool.clone();
            let key = key.clone();
            async move {
                ops_brain::repo::handoff_repo::create_machine_handoff(
                    &pool,
                    "Test-Host1",
                    to,
                    "high",
                    "action",
                    "[auto] shared check FAIL",
                    "body",
                    None,
                    Some(&key),
                )
                .await
                .unwrap()
            }
        };

        let first = file("test-agent-alpha").await;
        let second = file("test-agent-beta").await;
        assert_ne!(
            first.id, second.id,
            "same key to a different recipient must file fresh"
        );
        assert_eq!(second.repeat_count, 0);

        // Same key to the SAME recipient (case-insensitive) still suppresses.
        let repeat = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            "Test-Agent-Alpha",
            "high",
            "action",
            "[auto] shared check FAIL",
            "body",
            None,
            Some(&key),
        )
        .await
        .unwrap();
        assert_eq!(repeat.id, first.id);
        assert_eq!(repeat.repeat_count, 1);

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(vec![first.id, second.id])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// The wake-poll query: agent match is case-insensitive, `since` filters
    /// on updated_at so dedupe bumps re-surface a still-firing monitor.
    #[tokio::test]
    async fn list_pending_since_cursor_resurfaces_dedupe_bumps() {
        let pool = pool().await;
        // Unique agent per run keeps this test isolated from real rows.
        let agent = format!("test-agent-{}", uuid::Uuid::now_v7().simple());
        let key = format!("test-check-{}", uuid::Uuid::now_v7());

        let filed = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            &agent,
            "high",
            "action",
            "[auto] test check FAIL",
            "body",
            None,
            Some(&key),
        )
        .await
        .unwrap();

        // Case-insensitive agent match, no cursor.
        let all = ops_brain::repo::handoff_repo::list_pending_for_agent(
            &pool,
            &agent.to_uppercase(),
            None,
            50,
        )
        .await
        .unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].id, filed.id);

        // Cursor after the filing hides it.
        let cursor = filed.updated_at;
        let after =
            ops_brain::repo::handoff_repo::list_pending_for_agent(&pool, &agent, Some(cursor), 50)
                .await
                .unwrap();
        assert!(after.is_empty());

        // A dedupe bump moves updated_at past the cursor → re-surfaces.
        ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Test-Host1",
            &agent,
            "high",
            "action",
            "[auto] test check FAIL",
            "body",
            None,
            Some(&key),
        )
        .await
        .unwrap();
        let resurfaced =
            ops_brain::repo::handoff_repo::list_pending_for_agent(&pool, &agent, Some(cursor), 50)
                .await
                .unwrap();
        assert_eq!(resurfaced.len(), 1);
        assert_eq!(resurfaced[0].id, filed.id);
        assert_eq!(resurfaced[0].repeat_count, 1);

        // Completed rows drop out of the poll.
        ops_brain::repo::handoff_repo::complete_handoff_with_commit(&pool, filed.id, None)
            .await
            .unwrap()
            .expect("complete should succeed");
        let done = ops_brain::repo::handoff_repo::list_pending_for_agent(&pool, &agent, None, 50)
            .await
            .unwrap();
        assert!(done.is_empty());

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(filed.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Cross-client safety gate (end-to-end through the search handler) =====
//
// The pure-logic partitioning is unit-tested in `src/tools/mod.rs`; these
// drive the whole `handle_search_knowledge` path so the withhold → acknowledge
// → audit-log contract is locked against a real OpsBrain + Postgres. FTS mode
// with a unique term keeps the embedding client out of the picture.
mod knowledge_safety_tests {
    use super::*;
    use ops_brain::tools::knowledge::{handle_search_knowledge, SearchKnowledgeParams};

    fn build_brain(pool: PgPool) -> ops_brain::tools::OpsBrain {
        ops_brain::tools::OpsBrain::new(pool, None)
    }

    fn extract_json(result: &rmcp::model::CallToolResult) -> serde_json::Value {
        let text = result
            .content
            .first()
            .expect("result has content")
            .as_text()
            .expect("content is text")
            .text
            .clone();
        serde_json::from_str(&text).expect("result body is JSON")
    }

    /// FTS single-table search scoped to `client_slug`, non-compact so bodies
    /// (and provenance) are present on allowed items.
    async fn search_as(
        brain: &ops_brain::tools::OpsBrain,
        query: &str,
        client_slug: &str,
        acknowledge: bool,
    ) -> serde_json::Value {
        let result = handle_search_knowledge(
            brain,
            SearchKnowledgeParams {
                query: Some(query.to_string()),
                mode: Some("fts".to_string()),
                tables: None,
                client_slug: Some(client_slug.to_string()),
                acknowledge_cross_client: Some(acknowledge),
                limit: Some(50),
                compact: Some(false),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(false), "search should not error");
        extract_json(&result)
    }

    #[tokio::test]
    async fn cross_client_gate_withhold_acknowledge_and_audit() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let suffix = Uuid::now_v7().simple().to_string();

        let client_a = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Alpha Gate Co",
            &format!("alphagate-{suffix}"),
            None,
        )
        .await
        .unwrap();
        let client_b = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Beta Gate Co",
            &format!("betagate-{suffix}"),
            None,
        )
        .await
        .unwrap();

        // Unique FTS term so only our entries match.
        let secret_term = format!("zqxgate{suffix}");
        let safe_term = format!("zqxsafe{suffix}");

        // Client A entry, NOT cross-client safe.
        let secret = ops_brain::repo::knowledge_repo::add_knowledge(
            &pool,
            &format!("gate {secret_term} secret"),
            "sensitive content for A only",
            Some("safety"),
            &[],
            Some(client_a.id),
            false,
            Some("CC-Stealth"),
        )
        .await
        .unwrap();

        // Client A entry that IS cross-client safe.
        let safe = ops_brain::repo::knowledge_repo::add_knowledge(
            &pool,
            &format!("gate {safe_term} shareable"),
            "content marked safe for all clients",
            Some("safety"),
            &[],
            Some(client_a.id),
            true,
            Some("CC-Stealth"),
        )
        .await
        .unwrap();

        // 1) Client B searches the secret term WITHOUT acknowledgment → withheld.
        let withheld = search_as(&brain, &secret_term, &client_b.slug, false).await;
        let items = withheld["knowledge"].as_array().expect("knowledge array");
        assert!(
            items.is_empty(),
            "secret entry must be withheld, got: {items:?}"
        );
        let notices = withheld["cross_client_withheld"]
            .as_array()
            .expect("withheld notices present");
        assert_eq!(notices.len(), 1);
        assert_eq!(notices[0]["count"], 1);
        assert_eq!(notices[0]["owning_client_slug"], client_a.slug);
        // Content must NOT leak anywhere in the withheld response.
        assert!(
            !withheld
                .to_string()
                .contains("sensitive content for A only"),
            "withheld content must not appear in the response"
        );

        // 2) Client B re-calls WITH acknowledgment → released, content present,
        //    provenance stamped to the owning client.
        let released = search_as(&brain, &secret_term, &client_b.slug, true).await;
        let items = released["knowledge"].as_array().expect("knowledge array");
        assert_eq!(
            items.len(),
            1,
            "acknowledged search should release the item"
        );
        assert_eq!(items[0]["id"], secret.id.to_string());
        assert_eq!(items[0]["content"], "sensitive content for A only");
        assert_eq!(items[0]["_client_slug"], client_a.slug);
        assert_eq!(items[0]["_client_name"], client_a.name);

        // 3) audit_log recorded the release for client B.
        let released_rows: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM audit_log
              WHERE entity_id = $1 AND requesting_client_id = $2
                AND action = 'released' AND tool_name = 'search_knowledge'",
        )
        .bind(secret.id)
        .bind(client_b.id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            released_rows >= 1,
            "a released audit_log row must exist for the acknowledged surfacing"
        );

        // 4) cross_client_safe=true entry passes for client B WITHOUT ack.
        let safe_resp = search_as(&brain, &safe_term, &client_b.slug, false).await;
        let items = safe_resp["knowledge"].as_array().expect("knowledge array");
        assert_eq!(items.len(), 1, "safe entry passes without acknowledgment");
        assert_eq!(items[0]["id"], safe.id.to_string());
        assert_eq!(items[0]["_client_slug"], client_a.slug);
        assert_eq!(items[0]["_client_name"], client_a.name);
        assert!(
            safe_resp.get("cross_client_withheld").is_none(),
            "safe entry must not produce a withheld notice"
        );

        // Cleanup: audit_log FKs requesting_client_id → clients, so purge it
        // before the clients. knowledge.client_id is ON DELETE SET NULL.
        sqlx::query("DELETE FROM audit_log WHERE entity_id = ANY($1)")
            .bind(vec![secret.id, safe.id])
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM knowledge WHERE id = ANY($1)")
            .bind(vec![secret.id, safe.id])
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM clients WHERE id = ANY($1)")
            .bind(vec![client_a.id, client_b.id])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// Step 8: an UNSCOPED search (no client_slug) applies no withholding but
    /// still stamps provenance on every item and carries the `_note` banner.
    #[tokio::test]
    async fn unscoped_search_injects_note_and_provenance() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let suffix = Uuid::now_v7().simple().to_string();

        let client_a = ops_brain::repo::client_repo::upsert_client(
            &pool,
            "Unscoped Co",
            &format!("unscoped-{suffix}"),
            None,
        )
        .await
        .unwrap();

        let term = format!("zqxunscoped{suffix}");
        let entry = ops_brain::repo::knowledge_repo::add_knowledge(
            &pool,
            &format!("open {term} entry"),
            "unscoped body",
            None,
            &[],
            Some(client_a.id),
            false,
            Some("CC-Stealth"),
        )
        .await
        .unwrap();

        // No client_slug → unscoped.
        let result = handle_search_knowledge(
            &brain,
            SearchKnowledgeParams {
                query: Some(term.clone()),
                mode: Some("fts".to_string()),
                tables: None,
                client_slug: None,
                acknowledge_cross_client: None,
                limit: Some(50),
                compact: Some(false),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        let resp = extract_json(&result);

        assert!(
            resp["_note"]
                .as_str()
                .unwrap_or_default()
                .contains("unscoped query"),
            "unscoped response must carry the _note banner, got: {resp}"
        );
        let items = resp["knowledge"].as_array().expect("knowledge array");
        assert_eq!(items.len(), 1);
        // Provenance is present even without a requesting client.
        assert_eq!(items[0]["_client_slug"], client_a.slug);
        assert_eq!(items[0]["_client_name"], client_a.name);
        assert!(
            resp.get("cross_client_withheld").is_none(),
            "unscoped query never withholds"
        );

        sqlx::query("DELETE FROM knowledge WHERE id = $1")
            .bind(entry.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM clients WHERE id = $1")
            .bind(client_a.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== bearer_auth middleware enforcement =====
//
// Drives the real middleware over a tiny router via tower::oneshot — no DB.
// Confirms caller classification (Full vs Machine), scope/path gating, and the
// 401/403/dev-mode boundaries.
mod auth_middleware_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::{get, post};
    use axum::{Extension, Router};
    use ops_brain::auth::{bearer_auth, AuthState, CallerClass, MachineToken};
    use std::sync::Arc;
    use tower::ServiceExt;

    const MAIN: &str = "main-secret-token-000000000000000000000000";
    const MACH: &str = "machine-secret-token-1111111111111111111111";

    async fn echo_caller(Extension(caller): Extension<CallerClass>) -> String {
        match caller {
            CallerClass::Full => "Full".to_string(),
            CallerClass::Machine(t) => format!("Machine:{}", t.from_agent),
        }
    }

    fn machine_token(scopes: Vec<&str>) -> MachineToken {
        MachineToken {
            token: MACH.to_string(),
            from_agent: "Test-Producer".to_string(),
            client: None,
            agents: vec!["CC-Test".to_string()],
            scopes: scopes.into_iter().map(String::from).collect(),
        }
    }

    fn app(state: AuthState) -> Router {
        Router::new()
            .route("/health", get(|| async { "OK" }))
            .route("/api/briefing", post(echo_caller))
            .route("/api/handoff", post(echo_caller))
            .route("/api/pending", get(echo_caller))
            .route("/mcp", post(echo_caller))
            .layer(from_fn_with_state(state, bearer_auth))
    }

    fn full_state(machine: Vec<MachineToken>) -> AuthState {
        AuthState {
            main_token: Some(MAIN.to_string()),
            machine_tokens: Arc::new(machine),
        }
    }

    async fn body_text(resp: axum::response::Response) -> String {
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX)
            .await
            .unwrap();
        String::from_utf8(bytes.to_vec()).unwrap()
    }

    fn req(method: &str, uri: &str, bearer: Option<&str>) -> Request<Body> {
        let mut b = Request::builder().method(method).uri(uri);
        if let Some(tok) = bearer {
            b = b.header("authorization", format!("Bearer {tok}"));
        }
        b.body(Body::empty()).unwrap()
    }

    #[tokio::test]
    async fn main_bearer_is_full() {
        let resp = app(full_state(vec![]))
            .oneshot(req("GET", "/api/pending", Some(MAIN)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "Full");
    }

    #[tokio::test]
    async fn machine_token_on_non_machine_path_is_forbidden() {
        // POST /api/briefing is not in the machine scope table.
        let resp = app(full_state(vec![machine_token(vec!["create", "read"])]))
            .oneshot(req("POST", "/api/briefing", Some(MACH)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn machine_token_wrong_scope_on_own_endpoint_is_forbidden() {
        // read-only token hitting POST /api/handoff (needs "create").
        let resp = app(full_state(vec![machine_token(vec!["read"])]))
            .oneshot(req("POST", "/api/handoff", Some(MACH)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn machine_token_on_granted_endpoint_is_machine() {
        let resp = app(full_state(vec![machine_token(vec!["create", "read"])]))
            .oneshot(req("GET", "/api/pending", Some(MACH)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "Machine:Test-Producer");
    }

    #[tokio::test]
    async fn garbage_bearer_is_unauthorized() {
        let resp = app(full_state(vec![]))
            .oneshot(req("GET", "/api/pending", Some("not-a-real-token")))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn missing_header_is_unauthorized() {
        let resp = app(full_state(vec![]))
            .oneshot(req("GET", "/api/pending", None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn no_main_token_is_dev_mode_full() {
        let state = AuthState {
            main_token: None,
            machine_tokens: Arc::new(vec![]),
        };
        let resp = app(state)
            .oneshot(req("GET", "/api/pending", None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "Full");
    }
}

// ===== api::create_handoff identity-binding branches =====
//
// Full router (routes + auth layer + a machine token) driven via oneshot with
// a real pool, so the machine caller class is produced by the real middleware.
mod api_handoff_tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::post;
    use axum::Router;
    use ops_brain::api::ApiState;
    use ops_brain::auth::{bearer_auth, AuthState, MachineToken};
    use std::sync::Arc;
    use tower::ServiceExt;

    const MAIN: &str = "main-secret-token-222222222222222222222222";
    const MACH: &str = "machine-secret-token-3333333333333333333333";

    fn app(pool: PgPool) -> Router {
        let api_state = Arc::new(ApiState { pool });
        let api_routes = Router::new()
            .route("/handoff", post(ops_brain::api::create_handoff))
            .with_state(api_state);
        let auth_state = AuthState {
            main_token: Some(MAIN.to_string()),
            machine_tokens: Arc::new(vec![MachineToken {
                token: MACH.to_string(),
                from_agent: "Test-Producer".to_string(),
                client: None,
                agents: vec!["Codex-HSR".to_string()],
                scopes: vec!["create".to_string(), "read".to_string()],
            }]),
        };
        Router::new()
            .nest("/api", api_routes)
            .layer(from_fn_with_state(auth_state, bearer_auth))
    }

    fn post_handoff(body: serde_json::Value) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/handoff")
            .header("authorization", format!("Bearer {MACH}"))
            .header("content-type", "application/json")
            .body(Body::from(body.to_string()))
            .unwrap()
    }

    #[tokio::test]
    async fn from_agent_mismatch_is_bad_request() {
        let resp = app(pool().await)
            .oneshot(post_handoff(serde_json::json!({
                "from_agent": "Someone-Else",
                "to_agent": "Codex-HSR",
                "title": "t",
                "body": "b"
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn missing_to_agent_is_bad_request() {
        let resp = app(pool().await)
            .oneshot(post_handoff(serde_json::json!({
                "title": "t",
                "body": "b"
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn to_agent_outside_allowlist_is_forbidden() {
        let resp = app(pool().await)
            .oneshot(post_handoff(serde_json::json!({
                "to_agent": "CC-Other",
                "title": "t",
                "body": "b"
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn happy_path_creates_machine_origin_handoff() {
        let pool = pool().await;
        let title = format!("machine handoff {}", Uuid::now_v7().simple());
        let resp = app(pool.clone())
            .oneshot(post_handoff(serde_json::json!({
                "to_agent": "Codex-HSR",
                "title": title,
                "body": "filed via REST"
            })))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::CREATED);

        // origin isn't in the response shape — verify it landed server-stamped.
        let origin: String = sqlx::query_scalar("SELECT origin FROM handoffs WHERE title = $1")
            .bind(&title)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(origin, "machine");

        sqlx::query("DELETE FROM handoffs WHERE title = $1")
            .bind(&title)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Exact agent-name matching on list filters =====
//
// `_` is legal in agent names; the list filters must use LOWER()=LOWER(), not
// ILIKE, or the `_` single-char wildcard over-matches sibling agents.
mod exact_agent_match_tests {
    use super::*;

    #[tokio::test]
    async fn list_handoffs_to_agent_filter_is_exact_not_wildcard() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        // Differ only at one position: literal '_' vs 'X'. Same length, so an
        // ILIKE '_' wildcard would match both; exact matching must not.
        let underscore_agent = format!("z{uniq}_x");
        let wildcard_agent = format!("z{uniq}Xx");

        let h_under = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            "CC-Stealth",
            Some(&underscore_agent),
            "normal",
            "action",
            "exact-match underscore",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        let h_wild = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            "CC-Stealth",
            Some(&wildcard_agent),
            "normal",
            "action",
            "exact-match wildcard",
            "body",
            None,
            None,
        )
        .await
        .unwrap();

        let results = ops_brain::repo::handoff_repo::list_handoffs(
            &pool,
            None,
            Some(&underscore_agent),
            None,
            None,
            false,
            50,
        )
        .await
        .unwrap();

        assert_eq!(
            results.len(),
            1,
            "filter must match only the exact underscore agent, not the wildcard sibling"
        );
        assert_eq!(
            results[0].to_agent.as_deref(),
            Some(underscore_agent.as_str())
        );

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(vec![h_under.id, h_wild.id])
            .execute(&pool)
            .await
            .unwrap();
    }
}
