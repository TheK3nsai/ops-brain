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
            None,
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
            None,
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
            None,
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
            None,
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
            ("Claude-Laptop", Some("Claude-Cloud")),
            ("Codex-Buildbox", Some("Gemini-Buildbox")),
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

    /// in_reply_to threads a child handoff to a parent, and the reply reaches
    /// the parent's sender through the ordinary open queue.
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

        let queue = ops_brain::repo::handoff_repo::list_open_handoffs(
            &pool,
            Some(&alice),
            None,
            Some("action"),
            false,
            false,
            true,
            None,
            10,
        )
        .await
        .unwrap();
        assert_eq!(
            queue.iter().map(|h| h.id).collect::<Vec<_>>(),
            vec![reply.id]
        );

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
}

// ===== v5 exact retrieval + consolidated browse =====

mod v5_surface_tests {
    use super::*;
    use ops_brain::tools::coordination::{handle_get_handoff, GetHandoffParams};
    use ops_brain::tools::OpsBrain;

    #[tokio::test]
    async fn get_handoff_returns_one_full_record() {
        let pool = pool().await;
        let brain = OpsBrain::new(pool.clone());
        let handoff = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            "Codex-Laptop",
            Some("Claude-Buildbox"),
            "normal",
            "action",
            "exact v5 retrieval",
            "full body returned by exact UUID",
            None,
            None,
        )
        .await
        .unwrap();

        let result = handle_get_handoff(
            &brain,
            GetHandoffParams {
                handoff_id: handoff.id.to_string(),
            },
            ops_brain::auth::Visibility::Full,
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        let value = result.structured_content.expect("structured handoff");
        assert_eq!(value["id"], handoff.id.to_string());
        assert_eq!(value["body"], "full body returned by exact UUID");

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(handoff.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn rest_briefing_is_stateless() {
        let pool = pool().await;
        let value = ops_brain::tools::briefings::generate_briefing_inner(
            &pool,
            "daily",
            ops_brain::tools::briefings::DEFAULT_OPERATOR,
        )
        .await
        .unwrap();
        assert_eq!(value["briefing_type"], "daily");
        assert!(value.get("briefing_id").is_none());
        let table: Option<String> =
            sqlx::query_scalar("SELECT to_regclass('public.briefings')::text")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(table.is_none(), "briefings table should be removed in v5");
    }
}

// ===== Audit Log Repo =====

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
        ops_brain::tools::OpsBrain::new(pool)
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
            None,
            ops_brain::auth::Visibility::Full,
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
                agent_name: "Claude-Laptop".to_string(),
            },
            None,
            ops_brain::auth::Visibility::Full,
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
            "Claude-Laptop",
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
            None,
            ops_brain::auth::Visibility::Full,
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        let text = extract_text(&result);
        assert!(text.contains("accepted visibility smoke"));
        let structured = result
            .structured_content
            .as_ref()
            .expect("check_in should return structured JSON");
        assert_eq!(structured["open_handoffs_to_you"]["accepted_count"], 1);

        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(h.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    async fn file_action(pool: &PgPool, from: &str, to: Option<&str>, title: &str) -> Uuid {
        ops_brain::repo::handoff_repo::create_handoff(
            pool, from, to, "normal", "action", title, "body", None, None,
        )
        .await
        .unwrap()
        .id
    }

    async fn check_in_json(pool: PgPool, agent: &str) -> serde_json::Value {
        let brain = build_brain(pool);
        let result = ops_brain::tools::check_in::handle_check_in(
            &brain,
            ops_brain::tools::check_in::CheckInParams {
                agent_name: agent.to_string(),
            },
            None,
            ops_brain::auth::Visibility::Full,
        )
        .await;
        assert_eq!(result.is_error, Some(false));
        result
            .structured_content
            .clone()
            .expect("check_in returns structured JSON")
    }

    /// A handoff an agent files to itself and accepts is a lock record (a host's
    /// row claims). It must not take an action slot or spend a wake, but it
    /// stays counted, and a *pending* self-addressed row is still work.
    #[tokio::test]
    async fn check_in_and_wake_skip_accepted_self_claims() {
        let pool = pool().await;
        let agent = format!("Codex-Claim-{}", Uuid::now_v7().simple());

        let claim = file_action(&pool, &agent, Some(&agent), "self claim").await;
        ops_brain::repo::handoff_repo::accept_handoff(&pool, claim)
            .await
            .unwrap()
            .expect("accept should succeed");
        let note = file_action(&pool, &agent, Some(&agent), "pending note to self").await;
        let inbound = file_action(&pool, "Claude-Laptop", Some(&agent), "accepted inbound").await;
        ops_brain::repo::handoff_repo::accept_handoff(&pool, inbound)
            .await
            .unwrap()
            .expect("accept should succeed");

        let json = check_in_json(pool.clone(), &agent).await;
        let section = &json["open_handoffs_to_you"];
        let listed: Vec<&str> = section["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|h| h["title"].as_str().unwrap())
            .collect();
        assert!(!listed.contains(&"self claim"), "lock record took a slot");
        assert!(listed.contains(&"pending note to self"));
        assert!(listed.contains(&"accepted inbound"));
        assert_eq!(section["self_claims_held"]["count"], 1);
        assert_eq!(section["self_claims_held"]["oldest_age_days"], 0);

        let woken = ops_brain::repo::handoff_repo::list_pending_for_agent(&pool, &agent, None, 50)
            .await
            .unwrap();
        let woken: Vec<Uuid> = woken.iter().map(|h| h.id).collect();
        assert!(!woken.contains(&claim), "lock record would spend a wake");
        assert!(woken.contains(&note) && woken.contains(&inbound));

        // list_handoffs stays literal: the claim is still there to be found.
        let all = ops_brain::repo::handoff_repo::list_handoffs(
            &pool,
            Some("accepted"),
            Some(&agent),
            None,
            None,
            false,
            None,
            50,
        )
        .await
        .unwrap();
        assert!(all.iter().any(|h| h.id == claim));

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(vec![claim, note, inbound])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// Past the 20-row action cap, check_in must SAY it truncated. The page
    /// silently dropping row 21 is the failure this pins: an agent reading
    /// `count: 20` has no way to tell a full queue from a complete one.
    #[tokio::test]
    async fn check_in_reports_has_more_past_the_action_cap() {
        let pool = pool().await;
        let agent = format!("Codex-Cap-{}", Uuid::now_v7().simple());

        let mut ids = Vec::new();
        for i in 0..21 {
            ids.push(
                file_action(&pool, "Claude-Laptop", Some(&agent), &format!("capped {i}")).await,
            );
        }

        let json = check_in_json(pool.clone(), &agent).await;
        assert_eq!(json["open_handoffs_to_you"]["count"], 20);
        assert_eq!(json["open_handoffs_to_you"]["has_more"], true);

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn check_in_reports_no_more_under_the_action_cap() {
        let pool = pool().await;
        let agent = format!("Codex-Under-{}", Uuid::now_v7().simple());

        // Unaddressed rows now land in every agent's check_in, so an unrelated
        // backlog of them could legitimately fill this page. Assert the
        // precondition so a failure here reads as "dirty test DB", not "bug".
        let ambient: i64 = sqlx::query_scalar(
            "SELECT count(*) FROM handoffs
              WHERE status = 'pending' AND to_agent IS NULL
                AND (category = 'action' OR created_at > now() - interval '7 days')",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            ambient < 4,
            "test DB holds {ambient} open unaddressed handoffs — too many to test the under-cap page"
        );

        let ids = vec![
            file_action(&pool, "Claude-Laptop", Some(&agent), "one").await,
            file_action(&pool, "Claude-Laptop", Some(&agent), "two").await,
        ];

        let json = check_in_json(pool.clone(), &agent).await;
        assert_eq!(json["open_handoffs_to_you"]["has_more"], false);
        assert_eq!(json["recent_notifications"]["has_more"], false);

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(&ids)
            .execute(&pool)
            .await
            .unwrap();
    }
    /// Only a notify can be filed without a recipient. It reaches every other
    /// agent's notification page, never the sender's, and never the wake-shim
    /// poll, so one announcement can't wake every host. An unaddressed action
    /// (a legacy row) reaches nobody's queue.
    #[tokio::test]
    async fn check_in_surfaces_broadcast_notifies_to_everyone_but_the_sender() {
        let pool = pool().await;
        let sender = format!("Claude-Sender-{}", Uuid::now_v7().simple());
        let other = format!("Claude-Other-{}", Uuid::now_v7().simple());

        let notify = ops_brain::repo::handoff_repo::create_handoff(
            &pool, &sender, None, "normal", "notify", "announce", "body", None, None,
        )
        .await
        .unwrap()
        .id;
        let legacy_action = file_action(&pool, &sender, None, "legacy unaddressed").await;
        let has = |v: &serde_json::Value, section: &str, id: Uuid| {
            v[section]["items"]
                .as_array()
                .unwrap()
                .iter()
                .any(|h| h["id"].as_str() == Some(&id.to_string()))
        };

        let theirs = check_in_json(pool.clone(), &other).await;
        assert!(has(&theirs, "recent_notifications", notify), "{theirs}");
        assert!(
            !has(&theirs, "open_handoffs_to_you", legacy_action),
            "{theirs}"
        );

        let mine = check_in_json(pool.clone(), &sender).await;
        assert!(!has(&mine, "recent_notifications", notify), "{mine}");

        let woken = ops_brain::repo::handoff_repo::list_pending_for_agent(&pool, &other, None, 50)
            .await
            .unwrap();
        assert!(woken.is_empty(), "a broadcast must not wake a host");

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(vec![notify, legacy_action])
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
    use ops_brain::tools::coordination::{handle_create_handoff, CreateHandoffParams};

    fn build_brain(pool: PgPool) -> ops_brain::tools::OpsBrain {
        ops_brain::tools::OpsBrain::new(pool)
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
                from_agent: "Claude-Laptop".to_string(),
                to_agent: None,
                priority: None,
                category: None,
                title: "smoke".to_string(),
                body: "body".to_string(),
                context: None,
                in_reply_to: Some("not-a-uuid".to_string()),
                complete_parent: None,
                commit_hash: None,
            },
            None,
            ops_brain::auth::Visibility::Full,
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(
            text.contains("Invalid in_reply_to UUID"),
            "expected in_reply_to validation error, got: {text}"
        );
    }

    fn handoff_params(from_agent: &str) -> CreateHandoffParams {
        CreateHandoffParams {
            from_agent: from_agent.to_string(),
            to_agent: Some("Claude-Cloud".to_string()),
            priority: None,
            category: Some("notify".to_string()),
            title: "identity-binding smoke".to_string(),
            body: "body".to_string(),
            context: None,
            in_reply_to: None,
            complete_parent: None,
            commit_hash: None,
        }
    }

    #[tokio::test]
    async fn create_handoff_bound_token_rejects_mismatched_from_agent() {
        // A per-agent token bound to Claude-Laptop cannot file as Claude-Cloud.
        let brain = build_brain(pool().await);
        let result = handle_create_handoff(
            &brain,
            handoff_params("Claude-Cloud"),
            Some("Claude-Laptop"),
            ops_brain::auth::Visibility::Full,
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(
            text.contains("Claude-Laptop") && text.contains("Claude-Cloud"),
            "expected identity-mismatch error naming both slugs, got: {text}"
        );
    }

    #[tokio::test]
    async fn create_handoff_bound_token_allows_matching_from_agent_case_insensitive() {
        let brain = build_brain(pool().await);
        let result = handle_create_handoff(
            &brain,
            handoff_params("claude-laptop"),
            Some("Claude-Laptop"),
            ops_brain::auth::Visibility::Full,
        )
        .await;
        assert_eq!(
            result.is_error,
            Some(false),
            "matching (case-insensitive) identity must be accepted: {}",
            extract_text(&result)
        );
    }

    #[tokio::test]
    async fn create_handoff_unbound_caller_files_any_identity() {
        // The main bearer (bound = None) keeps the pre-tokens behavior.
        let brain = build_brain(pool().await);
        let result = handle_create_handoff(
            &brain,
            handoff_params("Claude-Anything"),
            None,
            ops_brain::auth::Visibility::Full,
        )
        .await;
        assert_eq!(
            result.is_error,
            Some(false),
            "unbound caller must file freely: {}",
            extract_text(&result)
        );
    }

    fn params(from: &str, to: Option<&str>, category: &str) -> CreateHandoffParams {
        CreateHandoffParams {
            from_agent: from.to_string(),
            to_agent: to.map(str::to_string),
            priority: None,
            category: Some(category.to_string()),
            title: "surface smoke".to_string(),
            body: "body".to_string(),
            context: None,
            in_reply_to: None,
            complete_parent: None,
            commit_hash: None,
        }
    }

    async fn create(
        brain: &ops_brain::tools::OpsBrain,
        p: CreateHandoffParams,
    ) -> rmcp::model::CallToolResult {
        handle_create_handoff(brain, p, None, ops_brain::auth::Visibility::Full).await
    }

    fn body(result: &rmcp::model::CallToolResult) -> serde_json::Value {
        assert_eq!(result.is_error, Some(false), "{}", extract_text(result));
        result
            .structured_content
            .clone()
            .expect("create_handoff returns structured JSON")
    }

    async fn status_of(pool: &PgPool, id: Uuid) -> (String, Option<String>) {
        let h = ops_brain::repo::handoff_repo::get_handoff(pool, id)
            .await
            .unwrap()
            .expect("handoff exists");
        (h.status, h.commit_hash)
    }

    /// An action nobody is named on is work nobody owns, so it is refused.
    /// An announcement can still go to everyone.
    #[tokio::test]
    async fn only_a_notify_can_be_broadcast() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let sender = format!("Claude-Sender-{}", Uuid::now_v7().simple());

        let action = create(&brain, params(&sender, None, "action")).await;
        assert_eq!(action.is_error, Some(true));
        assert!(
            extract_text(&action).contains("to_agent is required"),
            "{}",
            extract_text(&action)
        );

        let notify = body(&create(&brain, params(&sender, None, "notify")).await);
        assert!(notify["to_agent"].is_null());

        sqlx::query("DELETE FROM handoffs WHERE from_agent = $1")
            .bind(&sender)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// The common closing motion in one call: the recipient's reply completes
    /// the parent and records the commit on it.
    #[tokio::test]
    async fn reply_from_the_recipient_completes_the_parent() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let suffix = Uuid::now_v7().simple().to_string();
        let (alice, bob) = (format!("Alice-{suffix}"), format!("Bob-{suffix}"));

        let parent = body(&create(&brain, params(&alice, Some(&bob), "action")).await);
        let parent_id = Uuid::parse_str(parent["id"].as_str().unwrap()).unwrap();

        let mut reply = params(&bob, Some(&alice), "notify");
        reply.in_reply_to = Some(parent_id.to_string());
        reply.complete_parent = Some(true);
        reply.commit_hash = Some(" abc1234 ".to_string());
        let reply = body(&create(&brain, reply).await);

        assert_eq!(reply["_parent_completed"], true);
        assert!(reply.get("_warning").is_none(), "{reply}");
        assert_eq!(
            status_of(&pool, parent_id).await,
            ("completed".to_string(), Some("abc1234".to_string()))
        );

        // A second closing reply still files, and says the parent was closed.
        let mut again = params(&bob, Some(&alice), "notify");
        again.in_reply_to = Some(parent_id.to_string());
        again.complete_parent = Some(true);
        let again = body(&create(&brain, again).await);
        assert_eq!(again["_parent_completed"], false);
        assert!(
            again["_warning"]
                .as_str()
                .unwrap()
                .contains("not completed"),
            "{again}"
        );

        sqlx::query("DELETE FROM handoffs WHERE from_agent = ANY($1)")
            .bind(vec![alice, bob])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// A sender following up on its own thread must not close the
    /// recipient's open work with a stray flag.
    #[tokio::test]
    async fn complete_parent_only_closes_work_addressed_to_the_replier() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let suffix = Uuid::now_v7().simple().to_string();
        let (alice, bob) = (format!("Alice-{suffix}"), format!("Bob-{suffix}"));

        let parent = body(&create(&brain, params(&alice, Some(&bob), "action")).await);
        let parent_id = Uuid::parse_str(parent["id"].as_str().unwrap()).unwrap();

        let mut follow_up = params(&alice, Some(&bob), "notify");
        follow_up.in_reply_to = Some(parent_id.to_string());
        follow_up.complete_parent = Some(true);
        let follow_up = body(&create(&brain, follow_up).await);

        assert_eq!(follow_up["_parent_completed"], false);
        assert!(
            follow_up["id"].as_str().is_some(),
            "the reply is still filed"
        );
        assert_eq!(status_of(&pool, parent_id).await.0, "pending");

        sqlx::query("DELETE FROM handoffs WHERE from_agent = ANY($1)")
            .bind(vec![alice, bob])
            .execute(&pool)
            .await
            .unwrap();
    }

    /// A family member that accepted work addressed to the family base slug
    /// closes it by reply, and slug case does not matter.
    #[tokio::test]
    async fn complete_parent_honours_the_recorded_acceptor() {
        let pool = pool().await;
        let brain = build_brain(pool.clone());
        let suffix = Uuid::now_v7().simple().to_string();
        let (alice, family) = (format!("Alice-{suffix}"), format!("Fam-{suffix}"));
        let member = format!("{family}-Builder");

        let parent = body(&create(&brain, params(&alice, Some(&family), "action")).await);
        let parent_id = Uuid::parse_str(parent["id"].as_str().unwrap()).unwrap();
        ops_brain::repo::handoff_repo::accept_handoff_as(&pool, parent_id, Some(&member))
            .await
            .unwrap()
            .expect("pending parent accepts");

        let mut reply = params(&member.to_lowercase(), Some(&alice), "notify");
        reply.in_reply_to = Some(parent_id.to_string());
        reply.complete_parent = Some(true);
        let reply = body(&create(&brain, reply).await);
        assert_eq!(reply["_parent_completed"], true, "{reply}");
        assert_eq!(status_of(&pool, parent_id).await.0, "completed");

        sqlx::query("DELETE FROM handoffs WHERE from_agent = ANY($1)")
            .bind(vec![alice, member.to_lowercase()])
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn complete_parent_and_commit_hash_need_their_companions() {
        let brain = build_brain(pool().await);

        let mut orphan = params("Claude-Laptop", Some("Claude-Cloud"), "notify");
        orphan.complete_parent = Some(true);
        let orphan = create(&brain, orphan).await;
        assert_eq!(orphan.is_error, Some(true));
        assert!(extract_text(&orphan).contains("needs in_reply_to"));

        let mut hash_only = params("Claude-Laptop", Some("Claude-Cloud"), "notify");
        hash_only.in_reply_to = Some(Uuid::now_v7().to_string());
        hash_only.commit_hash = Some("abc1234".to_string());
        let hash_only = create(&brain, hash_only).await;
        assert_eq!(hash_only.is_error, Some(true));
        assert!(extract_text(&hash_only).contains("set complete_parent"));
    }

    /// A recipient nobody holds a credential for is a typo, a retired seat or
    /// a machine producer's sender name. Each one sits unread, so the bus
    /// warns from the operator's static bindings and still writes the row.
    #[tokio::test]
    async fn create_handoff_warns_on_a_recipient_no_credential_is_bound_to() {
        let pool = pool().await;
        let suffix = Uuid::now_v7().simple().to_string();
        let sender = format!("Claude-Laptop-{suffix}");
        let brain = build_brain(pool.clone()).with_known_agents(vec![
            "Codex-Orion".to_string(),
            "Grok-Bot-*".to_string(),
            "codex-orion".to_string(),
        ]);

        let typo = body(&create(&brain, params(&sender, Some("Codex-Orian"), "notify")).await);
        let warning = typo["_warning"]
            .as_str()
            .unwrap_or_else(|| panic!("expected a _warning on an unbound recipient: {typo}"));
        assert!(
            warning.contains("'Codex-Orian'") && warning.contains("no agent or poller"),
            "warning must name the slug and the failure mode: {warning}"
        );
        assert!(
            warning.contains("bound slugs: Codex-Orion, Grok-Bot-*"),
            "warning must list the deduplicated bound slugs: {warning}"
        );
        assert!(
            typo["id"].as_str().is_some(),
            "the handoff is still created"
        );

        for bound in ["codex-orion", "Grok-Bot", "Grok-Bot-Builder"] {
            let ok = body(&create(&brain, params(&sender, Some(bound), "notify")).await);
            assert!(ok.get("_warning").is_none(), "{bound} is bound: {ok}");
        }

        // A caller kept outside client data gets no roster.
        let restricted = handle_create_handoff(
            &brain,
            params(&sender, Some("Codex-Orian"), "notify"),
            Some(&sender),
            ops_brain::auth::Visibility::Restricted(&sender),
        )
        .await;
        assert!(body(&restricted).get("_warning").is_none());

        // With no bindings configured there is nothing to check against.
        let unbound = build_brain(pool.clone());
        let quiet = body(&create(&unbound, params(&sender, Some("Codex-Orian"), "notify")).await);
        assert!(quiet.get("_warning").is_none(), "{quiet}");

        sqlx::query("DELETE FROM handoffs WHERE from_agent = $1")
            .bind(&sender)
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
    use ops_brain::auth::{bearer_auth, AgentToken, AuthState, CallerClass, MachineToken};
    use std::sync::Arc;
    use tower::ServiceExt;

    const MAIN: &str = "main-secret-token-000000000000000000000000";
    const MACH: &str = "machine-secret-token-1111111111111111111111";
    const AGENT: &str = "agent-secret-token-22222222222222222222222";

    async fn echo_caller(Extension(caller): Extension<CallerClass>) -> String {
        match caller {
            CallerClass::Full => "Full".to_string(),
            CallerClass::Machine(t) => format!("Machine:{}", t.from_agent),
            CallerClass::Agent(t) => format!("Agent:{}", t.from_agent),
        }
    }

    fn machine_token(scopes: Vec<&str>) -> MachineToken {
        MachineToken {
            token: MACH.to_string(),
            from_agent: "Test-Producer".to_string(),
            client: None,
            agents: vec!["Claude-Test".to_string()],
            scopes: scopes.into_iter().map(String::from).collect(),
        }
    }

    fn agent_token() -> AgentToken {
        AgentToken {
            token: AGENT.to_string(),
            from_agent: "Claude-Test".to_string(),
            client: None,
            exclude_client_data: false,
        }
    }

    fn app(state: AuthState) -> Router {
        Router::new()
            .route("/health", get(|| async { "OK" }))
            .route("/ready", get(|| async { "READY" }))
            .route("/api/briefing", post(echo_caller))
            .route("/api/handoff", post(echo_caller))
            .route("/api/pending", get(echo_caller))
            .route("/mcp", post(echo_caller))
            .route("/live", get(echo_caller))
            .layer(from_fn_with_state(state, bearer_auth))
    }

    fn full_state(machine: Vec<MachineToken>) -> AuthState {
        state_with(machine, vec![])
    }

    fn state_with(machine: Vec<MachineToken>, agents: Vec<AgentToken>) -> AuthState {
        AuthState {
            main_token: Some(MAIN.to_string()),
            machine_tokens: Arc::new(machine),
            agent_tokens: Arc::new(agents),
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
    async fn readiness_probe_is_public() {
        let resp = app(full_state(vec![]))
            .oneshot(req("GET", "/ready", None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "READY");
    }

    #[tokio::test]
    async fn no_main_token_is_dev_mode_full() {
        let state = AuthState {
            main_token: None,
            machine_tokens: Arc::new(vec![]),
            agent_tokens: Arc::new(vec![]),
        };
        let resp = app(state)
            .oneshot(req("GET", "/api/pending", None))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "Full");
    }

    // ----- per-agent token routing -----

    #[tokio::test]
    async fn agent_token_on_mcp_is_agent() {
        let resp = app(state_with(vec![], vec![agent_token()]))
            .oneshot(req("POST", "/mcp", Some(AGENT)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "Agent:Claude-Test");
    }

    #[tokio::test]
    async fn agent_token_on_retired_live_path_is_forbidden() {
        // The live lane is gone; an old launcher still pointed at /live must be
        // refused by auth, not reach whatever a future route mounts there.
        let resp = app(state_with(vec![], vec![agent_token()]))
            .oneshot(req("GET", "/live", Some(AGENT)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn agent_token_on_rest_handoff_is_forbidden() {
        // Agent tokens reach the MCP surface only — never the REST ingestion path.
        let resp = app(state_with(vec![], vec![agent_token()]))
            .oneshot(req("POST", "/api/handoff", Some(AGENT)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn agent_token_on_rest_pending_is_forbidden() {
        let resp = app(state_with(vec![], vec![agent_token()]))
            .oneshot(req("GET", "/api/pending", Some(AGENT)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn main_bearer_still_full_on_mcp_alongside_agent_tokens() {
        // The main bearer stays unbound (operator break-glass) even when
        // per-agent tokens are configured.
        let resp = app(state_with(vec![], vec![agent_token()]))
            .oneshot(req("POST", "/mcp", Some(MAIN)))
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(body_text(resp).await, "Full");
    }
}

mod readiness_tests {
    use super::*;
    use axum::extract::State;
    use axum::http::StatusCode;
    use ops_brain::api::ApiState;
    use std::sync::Arc;

    #[tokio::test]
    async fn ready_checks_the_database() {
        let state = Arc::new(ApiState { pool: pool().await });
        assert_eq!(ops_brain::api::ready(State(state)).await, StatusCode::OK);
    }
}

// ===== Bounded-page completeness metadata =====

mod pagination_surface_tests {
    use super::*;
    use axum::extract::{Extension, Query, State};
    use ops_brain::api::{ApiState, PendingQuery};
    use ops_brain::auth::CallerClass;
    use ops_brain::tools::coordination::{handle_list_handoffs, ListHandoffsParams};
    use ops_brain::tools::search::{handle_search_bus, SearchBusParams};
    use ops_brain::tools::OpsBrain;
    use std::sync::Arc;

    #[tokio::test]
    async fn handoff_lists_report_probe_rows_and_limit_clamps() {
        let pool = pool().await;
        let brain = OpsBrain::new(pool.clone());
        let suffix = Uuid::now_v7().simple().to_string();
        let sender = format!("pagination-sender-{suffix}");
        let reply_sender = format!("pagination-replier-{suffix}");

        let parent = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            &sender,
            None,
            "normal",
            "action",
            "pagination parent",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        for n in 0..3 {
            ops_brain::repo::handoff_repo::create_handoff(
                &pool,
                &reply_sender,
                Some(&sender),
                "normal",
                "action",
                &format!("pagination reply {n}"),
                "body",
                None,
                Some(parent.id),
            )
            .await
            .unwrap();
        }

        let listed = handle_list_handoffs(
            &brain,
            ListHandoffsParams {
                status: None,
                to_agent: None,
                from_agent: Some(reply_sender.clone()),
                category: None,
                include_notify: None,
                limit: Some(2),
                compact: Some(false),
            },
            ops_brain::auth::Visibility::Full,
        )
        .await
        .structured_content
        .expect("structured list response");
        assert_eq!(listed["handoffs"].as_array().unwrap().len(), 2);
        assert_eq!(listed["limit"], 2);
        assert_eq!(listed["limit_clamped"], false);
        assert_eq!(listed["has_more"], true);

        sqlx::query("DELETE FROM handoffs WHERE in_reply_to = $1")
            .bind(parent.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM handoffs WHERE id = $1")
            .bind(parent.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    /// Search and browse are capped pages that say when more exists, a
    /// compact result carries a snippet instead of the body, and a query
    /// whose words don't all match falls back to matching any of them.
    #[tokio::test]
    async fn search_bus_pages_compacts_and_falls_back_to_any_word() {
        let pool = pool().await;
        let brain = OpsBrain::new(pool.clone());
        let suffix = Uuid::now_v7().simple().to_string();
        let sender = format!("Pagination-Search-{suffix}");
        let term = format!("paginationterm{suffix}");
        let long_body = "pagination body ".repeat(40);
        for n in 0..3 {
            ops_brain::repo::handoff_repo::create_handoff(
                &pool,
                &sender,
                Some("Pagination-Peer"),
                "normal",
                if n == 0 { "notify" } else { "action" },
                &format!("{term} handoff {n}"),
                &long_body,
                None,
                None,
            )
            .await
            .unwrap();
        }
        let search = |query: &str, limit: i64, compact: Option<bool>| {
            let brain = brain.clone();
            let query = query.to_string();
            async move {
                handle_search_bus(
                    &brain,
                    SearchBusParams {
                        query: Some(query),
                        limit: Some(limit),
                        compact,
                    },
                    ops_brain::auth::Visibility::Full,
                )
                .await
                .structured_content
                .expect("structured search response")
            }
        };

        for query in ["*", term.as_str()] {
            let page = search(query, 2, None).await;
            assert_eq!(page["handoffs"].as_array().unwrap().len(), 2, "{query}");
            assert_eq!(page["limit"], 2);
            assert_eq!(page["limit_clamped"], false);
            assert_eq!(page["has_more"], true, "{query}");
        }

        let all = search(&term, 50, None).await;
        let rows = all["handoffs"].as_array().unwrap();
        assert_eq!(rows.len(), 3, "the notify is searchable too: {all}");
        assert_eq!(all["has_more"], false);
        assert!(rows[0].get("body").is_none() && rows[0].get("context").is_none());
        assert!(rows[0]["_snippet"].as_str().unwrap().len() <= 203);

        let full = search(&term, 50, Some(false)).await;
        assert_eq!(full["handoffs"][0]["body"], long_body);

        // "<term> nosuchword" matches nothing as all-words, then any-word.
        let fallback = search(&format!("{term} zzznosuchword{suffix}"), 50, None).await;
        assert_eq!(fallback["handoffs"].as_array().unwrap().len(), 3);
        let nothing = search(&format!("zzznosuchword{suffix}"), 50, None).await;
        assert!(nothing["handoffs"].as_array().unwrap().is_empty());
        // An exclusion is never loosened into "any word".
        let excluded = search(&format!("{term} -handoff"), 50, None).await;
        assert!(
            excluded["handoffs"].as_array().unwrap().is_empty(),
            "{excluded}"
        );

        sqlx::query("DELETE FROM handoffs WHERE from_agent = $1")
            .bind(&sender)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn pending_rest_reports_when_another_row_exists() {
        let pool = pool().await;
        let agent = format!("pagination-pending-{}", Uuid::now_v7().simple());
        for n in 0..3 {
            ops_brain::repo::handoff_repo::create_handoff(
                &pool,
                "Pagination-Producer",
                Some(&agent),
                "normal",
                "action",
                &format!("pending pagination {n}"),
                "body",
                None,
                None,
            )
            .await
            .unwrap();
        }

        let response = ops_brain::api::list_pending(
            State(Arc::new(ApiState { pool: pool.clone() })),
            Extension(CallerClass::Full),
            Query(PendingQuery {
                agent: agent.clone(),
                since: None,
                limit: Some(2),
            }),
        )
        .await
        .expect("pending response")
        .0;
        assert_eq!(response.count, 2);
        assert_eq!(response.limit, 2);
        assert!(!response.limit_clamped);
        assert!(response.has_more);

        sqlx::query("DELETE FROM handoffs WHERE to_agent = $1")
            .bind(&agent)
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Per-agent identity through the real HTTP MCP transport =====
//
// This locks the TypeId-sensitive path documented in GOTCHAS.md: axum auth
// inserts CallerClass into request parts, rmcp carries those parts into the
// tool-call Extensions bag, and the handler rejects forged provenance.
mod mcp_identity_transport_tests {
    use axum::middleware::from_fn_with_state;
    use ops_brain::auth::{bearer_auth, AgentToken, AuthState};
    use ops_brain::tools::OpsBrain;
    use rmcp::transport::streamable_http_server::{
        session::local::LocalSessionManager, StreamableHttpServerConfig, StreamableHttpService,
    };
    use sqlx::postgres::PgPoolOptions;
    use std::sync::Arc;

    const MAIN: &str = "main-secret-token-transport-00000000000000000";
    const AGENT: &str = "agent-secret-token-transport-111111111111111";

    #[tokio::test]
    async fn bound_identity_survives_transport_and_rejects_forged_sender() {
        // Lazy pool deliberately points nowhere: identity rejection happens
        // before the handler can touch PostgreSQL.
        let pool = PgPoolOptions::new()
            .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let pool_for_service = pool.clone();
        let mcp_service: StreamableHttpService<OpsBrain, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(OpsBrain::new(pool_for_service.clone())),
                Default::default(),
                StreamableHttpServerConfig::default().with_sse_keep_alive(None),
            );
        let auth_state = AuthState {
            main_token: Some(MAIN.to_string()),
            machine_tokens: Arc::new(vec![]),
            agent_tokens: Arc::new(vec![AgentToken {
                token: AGENT.to_string(),
                from_agent: "Codex-Laptop".to_string(),
                client: None,
                exclude_client_data: false,
            }]),
        };
        let app = axum::Router::new()
            .nest_service("/mcp", mcp_service)
            .layer(from_fn_with_state(auth_state, bearer_auth));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let init = client
            .post(format!("http://{addr}/mcp"))
            .bearer_auth(AGENT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .body(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"identity-test","version":"1.0"}}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(init.status(), reqwest::StatusCode::OK);
        let session_id = init
            .headers()
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();

        let initialized = client
            .post(format!("http://{addr}/mcp"))
            .bearer_auth(AGENT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("mcp-session-id", &session_id)
            .header("Mcp-Protocol-Version", "2025-06-18")
            .body(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(initialized.status(), reqwest::StatusCode::ACCEPTED);

        let response = client
            .post(format!("http://{addr}/mcp"))
            .bearer_auth(AGENT)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .header("mcp-session-id", &session_id)
            .header("Mcp-Protocol-Version", "2025-06-18")
            .body(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"create_handoff","arguments":{"from_agent":"Forged-Agent","to_agent":"Claude-Buildbox","title":"must reject","body":"must not reach the database"}}}"#)
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::OK);
        let body = response.text().await.unwrap();
        assert!(
            body.contains("does not match your token-bound agent") && body.contains("Codex-Laptop"),
            "identity mismatch must be rejected by the tool handler: {body}"
        );

        server.abort();
        let _ = server.await;
    }

    #[tokio::test]
    async fn client_data_exclusion_survives_transport() {
        // Same lazy pool: the restricted check_in refusal happens before any
        // query, so reaching it proves the flag rode the transport.
        let pool = PgPoolOptions::new()
            .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        let mcp_service: StreamableHttpService<OpsBrain, LocalSessionManager> =
            StreamableHttpService::new(
                move || Ok(OpsBrain::new(pool.clone())),
                Default::default(),
                StreamableHttpServerConfig::default().with_sse_keep_alive(None),
            );
        let auth_state = AuthState {
            main_token: Some(MAIN.to_string()),
            machine_tokens: Arc::new(vec![]),
            agent_tokens: Arc::new(vec![AgentToken {
                token: AGENT.to_string(),
                from_agent: "Test-Bot".to_string(),
                client: None,
                exclude_client_data: true,
            }]),
        };
        let app = axum::Router::new()
            .nest_service("/mcp", mcp_service)
            .layer(from_fn_with_state(auth_state, bearer_auth));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, app).await.unwrap();
        });

        let client = reqwest::Client::new();
        let post = |body: &'static str, session: Option<String>| {
            let mut req = client
                .post(format!("http://{addr}/mcp"))
                .bearer_auth(AGENT)
                .header("Content-Type", "application/json")
                .header("Accept", "application/json, text/event-stream");
            if let Some(s) = session {
                req = req
                    .header("mcp-session-id", s)
                    .header("Mcp-Protocol-Version", "2025-06-18");
            }
            req.body(body).send()
        };
        let init = post(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"restricted-test","version":"1.0"}}}"#, None)
            .await
            .unwrap();
        assert_eq!(init.status(), reqwest::StatusCode::OK);
        let session_id = init
            .headers()
            .get("mcp-session-id")
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        let initialized = post(
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
            Some(session_id.clone()),
        )
        .await
        .unwrap();
        assert_eq!(initialized.status(), reqwest::StatusCode::ACCEPTED);

        let response = post(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"check_in","arguments":{"agent_name":"Claude-Buildbox"}}}"#, Some(session_id))
            .await
            .unwrap();
        let body = response.text().await.unwrap();
        assert!(
            body.contains("excluded from client data") && body.contains("Test-Bot"),
            "restricted token must be refused another agent's queue: {body}"
        );

        server.abort();
        let _ = server.await;
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
                agents: vec!["Codex-Buildbox".to_string()],
                scopes: vec!["create".to_string(), "read".to_string()],
            }]),
            agent_tokens: Arc::new(vec![]),
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
                "to_agent": "Codex-Buildbox",
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
                "to_agent": "Claude-Other",
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
                "to_agent": "Codex-Buildbox",
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
            "Claude-Laptop",
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
            "Claude-Laptop",
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
            None,
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

// ===== REST error envelope =====
//
// Regression cover for handoff `01a0206b`: a client host's nightly sweep went dark for 5
// days because a rejected POST looked bodyless from the client. The rejection
// always named its field — it just rendered as `text/plain` while success
// rendered as JSON, so a JSON-parsing producer dropped it. These assert the
// wire shape a producer actually sees, not just the message text.
mod api_error_envelope_tests {
    use axum::body::{to_bytes, Body};
    use axum::http::{header, Request, StatusCode};
    use axum::middleware::from_fn_with_state;
    use axum::routing::post;
    use axum::{Extension, Router};
    use ops_brain::api::{create_handoff, ApiState};
    use ops_brain::auth::{bearer_auth, AuthState, CallerClass, MachineToken};
    use ops_brain::validation::MAX_TITLE_BYTES;
    use std::sync::Arc;
    use tower::ServiceExt;

    async fn json_body(resp: axum::response::Response) -> serde_json::Value {
        let ct = resp
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string();
        assert!(
            ct.starts_with("application/json"),
            "a producer parses this as JSON; got content-type {ct:?}"
        );
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        assert!(!bytes.is_empty(), "rejection body must not be empty");
        serde_json::from_slice(&bytes).expect("rejection body must parse as JSON")
    }

    async fn post_handoff(payload: serde_json::Value) -> axum::response::Response {
        let pool = super::common::test_pool().await;
        let app = Router::new()
            .route("/api/handoff", post(create_handoff))
            .layer(Extension(CallerClass::Full))
            .with_state(Arc::new(ApiState { pool }));

        app.oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/handoff")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(payload.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
    }

    /// That host's exact payload shape: 200 characters, 202 bytes because of one em
    /// dash. Rejected — and the rejection has to say which field and how big.
    #[tokio::test]
    async fn oversized_title_names_the_field_and_the_byte_count() {
        let title = format!("{}\u{2014}", "A".repeat(MAX_TITLE_BYTES - 1));
        assert_eq!(title.chars().count(), MAX_TITLE_BYTES);
        assert_eq!(title.len(), MAX_TITLE_BYTES + 2);

        let resp = post_handoff(serde_json::json!({
            "from_agent": "Claude-Test",
            "to_agent": "Claude-Test",
            "title": title,
            "body": "regression cover for 01a0206b",
        }))
        .await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["field"], "title");
        let msg = body["error"].as_str().unwrap();
        assert!(msg.contains("202") && msg.contains("200"), "got {msg}");
    }

    /// A missing required field is rejected by the extractor before any handler
    /// code runs. That path used to bypass the envelope entirely.
    #[tokio::test]
    async fn malformed_body_is_rejected_in_the_same_envelope() {
        let resp = post_handoff(serde_json::json!({"from_agent": "Claude-Test"})).await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert!(
            body["error"].as_str().unwrap().contains("title"),
            "extractor rejection should still name the missing field: {body}"
        );
    }

    #[tokio::test]
    async fn empty_title_names_the_field() {
        let resp = post_handoff(serde_json::json!({
            "from_agent": "Claude-Test",
            "to_agent": "Claude-Test",
            "title": "   ",
            "body": "x",
        }))
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        assert_eq!(json_body(resp).await["field"], "title");
    }

    #[tokio::test]
    async fn bad_priority_names_the_field_and_lists_valid_values() {
        let resp = post_handoff(serde_json::json!({
            "from_agent": "Claude-Test",
            "to_agent": "Claude-Test",
            "title": "t",
            "body": "b",
            "priority": "extreme",
        }))
        .await;
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = json_body(resp).await;
        assert_eq!(body["field"], "priority");
        assert!(body["error"].as_str().unwrap().contains("critical"));
    }

    // --- auth rejections: these were the genuinely bodyless ones ---

    const MAIN: &str = "main-secret-token-000000000000000000000000";
    const MACH: &str = "machine-secret-token-1111111111111111111111";

    fn guarded_app() -> Router {
        let state = AuthState {
            main_token: Some(MAIN.to_string()),
            machine_tokens: vec![MachineToken {
                token: MACH.to_string(),
                from_agent: "Test-Producer".to_string(),
                client: None,
                agents: vec!["Claude-Test".to_string()],
                scopes: vec!["handoff:create".to_string()],
            }]
            .into(),
            agent_tokens: vec![].into(),
        };
        Router::new()
            .route("/api/briefing", post(|| async { "OK" }))
            .layer(from_fn_with_state(state, bearer_auth))
    }

    async fn send(auth: Option<&str>) -> axum::response::Response {
        let mut b = Request::builder().method("POST").uri("/api/briefing");
        if let Some(t) = auth {
            b = b.header(header::AUTHORIZATION, format!("Bearer {t}"));
        }
        guarded_app()
            .oneshot(b.body(Body::empty()).unwrap())
            .await
            .unwrap()
    }

    #[tokio::test]
    async fn missing_credential_401_explains_the_expected_header() {
        let resp = send(None).await;
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        assert!(json_body(resp).await["error"]
            .as_str()
            .unwrap()
            .contains("Authorization"));
    }

    /// A bare 401 proves nothing without a positive control. Saying *which*
    /// 401 it is removes the need for one.
    #[tokio::test]
    async fn unrecognized_token_401_is_distinguishable_from_a_missing_one() {
        let missing = json_body(send(None).await).await;
        let wrong = json_body(send(Some("not-a-real-token")).await).await;
        assert_ne!(
            missing["error"], wrong["error"],
            "'no credential' and 'bad credential' must not be the same 401"
        );
    }

    #[tokio::test]
    async fn out_of_scope_403_names_the_missing_scope() {
        // POST /api/briefing is outside the machine scope table entirely.
        let resp = send(Some(MACH)).await;
        assert_eq!(resp.status(), StatusCode::FORBIDDEN);
        let msg = json_body(resp).await["error"].to_string();
        assert!(msg.contains("machine tokens"), "got {msg}");
    }
}

// ===== Briefing operator view =====
//
// The briefing is fleet-wide, so these tests never assert on totals or on the
// absence of *other* rows — every fixture uses a UUID-suffixed operator slug
// and UUID-suffixed titles, and assertions are about those strings only.
mod briefing_operator_view_tests {
    use super::*;
    use axum::body::{to_bytes, Body};
    use axum::http::{Request, StatusCode};
    use axum::routing::post;
    use axum::Router;
    use ops_brain::api::ApiState;
    use ops_brain::tools::briefings;
    use std::sync::Arc;
    use tower::ServiceExt;

    /// Push a handoff's `created_at` back so the age-threshold logic has
    /// something to bite on. There is no API for backdating — deliberately.
    async fn backdate(pool: &PgPool, id: Uuid, days: i64) {
        sqlx::query(
            "UPDATE handoffs SET created_at = now() - ($2 || ' days')::interval WHERE id = $1",
        )
        .bind(id)
        .bind(days.to_string())
        .execute(pool)
        .await
        .unwrap();
    }

    async fn file(pool: &PgPool, from: &str, to: Option<&str>, title: &str, days: i64) -> Uuid {
        let h = ops_brain::repo::handoff_repo::create_handoff(
            pool, from, to, "normal", "action", title, "body", None, None,
        )
        .await
        .unwrap();
        backdate(pool, h.id, days).await;
        h.id
    }

    async fn cleanup(pool: &PgPool, ids: &[Uuid]) {
        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(ids.to_vec())
            .execute(pool)
            .await
            .unwrap();
    }

    fn app(pool: PgPool) -> Router {
        // No auth layer: these exercise request validation, not the bearer
        // middleware (covered in auth_middleware_tests).
        Router::new()
            .route("/api/briefing", post(ops_brain::api::generate_briefing))
            .with_state(Arc::new(ApiState { pool }))
    }

    async fn post_briefing(
        pool: PgPool,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let resp = app(pool)
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/briefing")
                    .header("content-type", "application/json")
                    .body(Body::from(body.to_string()))
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap())
    }

    #[tokio::test]
    async fn operator_items_are_listed_oldest_first_with_ages() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        let operator = format!("Op-{uniq}");
        let old = format!("older operator item {uniq}");
        let new = format!("newer operator item {uniq}");

        let id_old = file(&pool, "Claude-Laptop", Some(&operator), &old, 26).await;
        let id_new = file(&pool, "Claude-Cloud", Some(&operator), &new, 3).await;

        let value = briefings::generate_briefing_inner(&pool, "daily", &operator)
            .await
            .unwrap();

        let items = value["waiting_on_you"]["items"].as_array().unwrap();
        assert_eq!(items.len(), 2, "only this operator's items may appear");
        assert_eq!(items[0]["title"], old, "oldest first");
        assert_eq!(items[0]["age_days"], 26);
        assert_eq!(items[1]["title"], new);
        assert_eq!(items[1]["age_days"], 3);
        assert_eq!(value["waiting_on_you"]["total"], 2);
        assert_eq!(value["waiting_on_you"]["truncated"], false);

        // And the markdown the operator actually reads carries the same order.
        let md = value["content"].as_str().unwrap();
        let (i_old, i_new) = (md.find(&old).unwrap(), md.find(&new).unwrap());
        assert!(i_old < i_new, "markdown must be oldest first too");
        assert!(md.contains(&format!("## Waiting on you ({operator})")));

        cleanup(&pool, &[id_old, id_new]).await;
    }

    #[tokio::test]
    async fn a_custom_operator_slug_is_honoured_over_the_default() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        let operator = format!("Op-{uniq}");
        let title = format!("custom-operator item {uniq}");
        // Deliberately same-day: the default-operator leg below asserts this
        // title is absent, which would also fail if the item were old enough
        // to surface under Stuck instead.
        let id = file(&pool, "Claude-Laptop", Some(&operator), &title, 0).await;

        let (status, value) = post_briefing(
            pool.clone(),
            serde_json::json!({"type": "daily", "operator": operator}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["operator"], operator);
        assert_eq!(value["waiting_on_you"]["total"], 1);
        assert_eq!(value["waiting_on_you"]["items"][0]["title"], title);

        // Omitting `operator` must still work — existing callers send only the
        // type — and must not pick this item up.
        let (status, value) =
            post_briefing(pool.clone(), serde_json::json!({"type": "daily"})).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(value["operator"], briefings::DEFAULT_OPERATOR);
        assert!(!value["content"].as_str().unwrap().contains(&title));

        cleanup(&pool, &[id]).await;
    }

    #[tokio::test]
    async fn self_addressed_and_machine_rows_are_counted_never_titled() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        let operator = format!("Op-{uniq}");
        let claimer = format!("Claude-Claimer-{uniq}");
        let self_title = format!("self-addressed claim {uniq}");
        let machine_title = format!("[auto] sweep finding {uniq}");

        // Old enough that only the exclusions keep them out of Stuck.
        let id_self = file(&pool, &claimer, Some(&claimer), &self_title, 10).await;
        let machine = ops_brain::repo::handoff_repo::create_machine_handoff(
            &pool,
            "Monitor",
            &format!("Claude-Target-{uniq}"),
            "normal",
            "action",
            &machine_title,
            "body",
            None,
            Some(&format!("key-{uniq}")),
        )
        .await
        .unwrap();
        backdate(&pool, machine.id, 10).await;

        let value = briefings::generate_briefing_inner(&pool, "daily", &operator)
            .await
            .unwrap();
        let md = value["content"].as_str().unwrap();

        assert!(
            !md.contains(&self_title),
            "self-addressed claims are a count, not a title"
        );
        assert!(
            !md.contains(&machine_title),
            "machine findings are a count, not a title"
        );
        // They are still in the shape counts, and in the legacy title lists the
        // JSON consumers already read.
        assert!(value["counts"]["self_addressed"]["count"].as_u64().unwrap() >= 1);
        assert!(
            value["counts"]["machine_findings"]["count"]
                .as_u64()
                .unwrap()
                >= 1
        );

        cleanup(&pool, &[id_self, machine.id]).await;
    }

    #[tokio::test]
    async fn stuck_lists_aged_items_for_other_agents_and_skips_fresh_ones() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        let operator = format!("Op-{uniq}");
        let target = format!("Claude-Target-{uniq}");
        let aged = format!("four days waiting {uniq}");
        let fresh = format!("filed today {uniq}");

        let id_aged = file(&pool, "Claude-Laptop", Some(&target), &aged, 4).await;
        let id_fresh = file(&pool, "Claude-Laptop", Some(&target), &fresh, 0).await;

        let value = briefings::generate_briefing_inner(&pool, "daily", &operator)
            .await
            .unwrap();
        let md = value["content"].as_str().unwrap();

        assert!(
            md.contains(&aged),
            "a 4-day-old pending item is past the 3-day threshold"
        );
        assert!(!md.contains(&fresh), "a same-day item is not stuck");
        assert!(
            md.contains(&format!("**{target}**")),
            "stuck items group by recipient"
        );

        let group = value["stuck"]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .find(|g| g["to_agent"] == target)
            .expect("group for the target agent");
        let titles: Vec<&str> = group["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["title"].as_str().unwrap())
            .collect();
        assert_eq!(titles, vec![aged.as_str()]);

        cleanup(&pool, &[id_aged, id_fresh]).await;
    }

    /// The unit tests hand-mirror `is_operator` / `is_self_addressed` in their
    /// fixture helper, so a broken SQL boolean would pass all of them. Only a
    /// real query proves the `LOWER() = LOWER()` comparison works.
    #[tokio::test]
    async fn operator_matching_is_case_insensitive_in_the_database() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        let filed_as = format!("Op-{uniq}");
        let requested_as = filed_as.to_lowercase();
        assert_ne!(filed_as, requested_as, "the test needs the cases to differ");
        let title = format!("case-folded operator item {uniq}");
        let id = file(&pool, "Claude-Laptop", Some(&filed_as), &title, 1).await;

        let value = briefings::generate_briefing_inner(&pool, "daily", &requested_as)
            .await
            .unwrap();
        assert_eq!(
            value["waiting_on_you"]["total"], 1,
            "a differently-cased slug must still match"
        );
        assert_eq!(value["waiting_on_you"]["items"][0]["title"], title);

        cleanup(&pool, &[id]).await;
    }

    #[tokio::test]
    async fn self_addressed_detection_is_case_insensitive_in_the_database() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        let agent = format!("Claude-Claimer-{uniq}");
        let title = format!("case-folded self claim {uniq}");
        // Same agent, different spelling: still self-addressed, so it must be
        // counted and must not be titled under Stuck.
        let id = file(&pool, &agent, Some(&agent.to_lowercase()), &title, 10).await;

        let value = briefings::generate_briefing_inner(&pool, "daily", &format!("Op-{uniq}"))
            .await
            .unwrap();
        assert!(
            !value["content"].as_str().unwrap().contains(&title),
            "a case-folded self-addressed row is still self-addressed"
        );

        cleanup(&pool, &[id]).await;
    }

    /// The change is only safe if the keys the off-repo mailer already reads
    /// survive it.
    #[tokio::test]
    async fn the_response_keeps_the_keys_existing_consumers_read() {
        let pool = pool().await;
        let value = briefings::generate_briefing_inner(&pool, "daily", "Operator")
            .await
            .unwrap();
        assert!(value["content"].is_string());
        assert!(value["generated_at"].is_string());
        assert_eq!(value["briefing_type"], "daily");
        for key in ["open_count", "pending_count", "accepted_count"] {
            assert!(
                value["handoffs"][key].is_u64(),
                "handoffs.{key} must survive"
            );
        }
        for key in ["pending_titles", "accepted_titles"] {
            assert!(
                value["handoffs"][key].is_array(),
                "handoffs.{key} must survive"
            );
        }
    }

    #[tokio::test]
    async fn notify_class_handoffs_never_reach_the_briefing() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        let operator = format!("Op-{uniq}");
        let title = format!("notify FYI {uniq}");
        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            "Claude-Laptop",
            Some(&operator),
            "normal",
            "notify",
            &title,
            "body",
            None,
            None,
        )
        .await
        .unwrap();

        let value = briefings::generate_briefing_inner(&pool, "daily", &operator)
            .await
            .unwrap();
        assert_eq!(
            value["waiting_on_you"]["total"], 0,
            "the briefing is the action queue; notify rows are not pending work"
        );
        assert!(!value["content"].as_str().unwrap().contains(&title));

        cleanup(&pool, &[h.id]).await;
    }

    /// An empty queue for a slug nothing has ever used must not read as
    /// "nothing is waiting on you" — that is the briefing lying.
    #[tokio::test]
    async fn an_unseen_operator_slug_is_called_out_in_the_markdown() {
        let pool = pool().await;
        let never_used = format!("Nobody-{}", Uuid::now_v7().simple());
        let value = briefings::generate_briefing_inner(&pool, "daily", &never_used)
            .await
            .unwrap();
        let md = value["content"].as_str().unwrap();
        assert_eq!(value["waiting_on_you"]["operator_seen"], false);
        assert!(
            md.contains(&format!("No handoff has ever named `{never_used}`")),
            "got {md}"
        );
        assert!(!md.contains("Nothing is waiting on you."));
    }

    #[tokio::test]
    async fn a_known_but_idle_operator_reads_as_a_clear_queue() {
        let pool = pool().await;
        let uniq = Uuid::now_v7().simple().to_string();
        let operator = format!("Op-{uniq}");
        // Seen on the bus, but with nothing open: a completed handoff.
        let h = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            "Claude-Laptop",
            Some(&operator),
            "normal",
            "action",
            &format!("already done {uniq}"),
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        ops_brain::repo::handoff_repo::complete_handoff_with_commit(&pool, h.id, None)
            .await
            .unwrap();

        let value = briefings::generate_briefing_inner(&pool, "daily", &operator)
            .await
            .unwrap();
        assert_eq!(value["waiting_on_you"]["operator_seen"], true);
        assert!(value["content"]
            .as_str()
            .unwrap()
            .contains("Nothing is waiting on you."));

        cleanup(&pool, &[h.id]).await;
    }

    #[tokio::test]
    async fn an_invalid_operator_slug_is_rejected_not_a_server_error() {
        let (status, body) = post_briefing(
            pool().await,
            serde_json::json!({"type": "daily", "operator": "not a slug!"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "got body {body}");
        assert_eq!(body["field"], "operator");
    }

    #[tokio::test]
    async fn an_empty_operator_slug_is_rejected() {
        let (status, body) = post_briefing(
            pool().await,
            serde_json::json!({"type": "daily", "operator": "  "}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "got body {body}");
        assert_eq!(body["field"], "operator");
    }
}

// ===== exclude_client_data tokens: the restricted boundary end to end =====
//
// A restricted caller sees only the handoffs it sent or received. These drive
// the real handlers against Postgres, with an unrestricted control on the
// same rows so a filter that hid everything couldn't pass.
mod restricted_visibility_tests {
    use super::*;
    use ops_brain::auth::Visibility;
    use ops_brain::tools::check_in::{handle_check_in, CheckInParams};
    use ops_brain::tools::coordination::{
        handle_accept_handoff, handle_create_handoff, handle_get_handoff, handle_list_handoffs,
        CreateHandoffParams, GetHandoffParams, ListHandoffsParams, UpdateHandoffStatusParams,
    };
    use ops_brain::tools::search::{handle_search_bus, SearchBusParams};

    fn text(result: &rmcp::model::CallToolResult) -> String {
        result
            .content
            .first()
            .expect("result has content")
            .as_text()
            .expect("content is text")
            .text
            .clone()
    }

    struct Fixture {
        pool: PgPool,
        brain: ops_brain::tools::OpsBrain,
        bot: String,
        term: String,
        to_bot: Uuid,
        from_bot: Uuid,
        others: Uuid,
        broadcast: Uuid,
    }

    async fn fixture() -> Fixture {
        let pool = pool().await;
        let brain = ops_brain::tools::OpsBrain::new(pool.clone());
        let suffix = Uuid::now_v7().simple().to_string();
        let bot = format!("Test-Bot-{}", &suffix[..12]);
        let term = format!("zqxrestrict{suffix}");
        let file = |from: String, to: Option<String>, title: String| {
            let pool = pool.clone();
            async move {
                ops_brain::repo::handoff_repo::create_handoff(
                    &pool,
                    &from,
                    to.as_deref(),
                    "normal",
                    "action",
                    &title,
                    "body",
                    None,
                    None,
                )
                .await
                .unwrap()
                .id
            }
        };
        let to_bot = file(
            "Claude-Laptop".into(),
            Some(bot.clone()),
            format!("to bot {term}"),
        )
        .await;
        let from_bot = file(
            bot.clone(),
            Some("Claude-Laptop".into()),
            format!("from bot {term}"),
        )
        .await;
        let others = file(
            "Claude-Buildbox".into(),
            Some("Claude-Laptop".into()),
            format!("others {term}"),
        )
        .await;
        let broadcast = file("Claude-Cloud".into(), None, format!("broadcast {term}")).await;

        Fixture {
            pool,
            brain,
            bot,
            term,
            to_bot,
            from_bot,
            others,
            broadcast,
        }
    }

    async fn cleanup(f: &Fixture) {
        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1) OR from_agent = $2")
            .bind(vec![f.to_bot, f.from_bot, f.others, f.broadcast])
            .bind(&f.bot)
            .execute(&f.pool)
            .await
            .unwrap();
    }

    fn search(term: &str) -> SearchBusParams {
        SearchBusParams {
            query: Some(term.to_string()),
            limit: Some(50),
            compact: Some(true),
        }
    }

    fn ids(v: &serde_json::Value, key: &str) -> Vec<Uuid> {
        v[key]
            .as_array()
            .unwrap_or_else(|| panic!("{key} array in {v}"))
            .iter()
            .map(|i| i["id"].as_str().unwrap().parse().unwrap())
            .collect()
    }

    #[tokio::test]
    async fn search_bus_returns_only_own_handoffs() {
        let f = fixture().await;
        let restricted = Visibility::Restricted(&f.bot);

        let r = handle_search_bus(&f.brain, search(&f.term), restricted).await;
        assert_eq!(r.is_error, Some(false), "{}", text(&r));
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        let mut handoffs = ids(&v, "handoffs");
        handoffs.sort();
        let mut expected = vec![f.to_bot, f.from_bot];
        expected.sort();
        assert_eq!(handoffs, expected);

        // The any-word fallback runs inside the same boundary.
        let loose = format!("{} zzznosuchword", f.term);
        let r = handle_search_bus(&f.brain, search(&loose), restricted).await;
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        let mut fallback = ids(&v, "handoffs");
        fallback.sort();
        assert_eq!(fallback, expected);

        // Browse takes the list path, not the FTS path.
        let mut browse = search("*");
        browse.limit = Some(200);
        let r = handle_search_bus(&f.brain, browse, restricted).await;
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        let browsed = ids(&v, "handoffs");
        assert!(browsed.contains(&f.to_bot) && browsed.contains(&f.from_bot));
        assert!(!browsed.contains(&f.others) && !browsed.contains(&f.broadcast));

        // Control: the same query unrestricted sees every row.
        let r = handle_search_bus(&f.brain, search(&f.term), Visibility::Full).await;
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        assert_eq!(ids(&v, "handoffs").len(), 4);

        cleanup(&f).await;
    }

    #[tokio::test]
    async fn slug_family_boundary_covers_members_only() {
        let f = fixture().await;
        let family = format!("{}-*", f.bot);
        let file = |from: String, to: String| {
            let pool = f.pool.clone();
            let term = f.term.clone();
            async move {
                ops_brain::repo::handoff_repo::create_handoff(
                    &pool,
                    &from,
                    Some(&to),
                    "normal",
                    "action",
                    &format!("family {term}"),
                    "body",
                    None,
                    None,
                )
                .await
                .unwrap()
                .id
            }
        };
        let member = file(format!("{}-Mail", f.bot), format!("{}-Coder", f.bot)).await;
        // Shares the base as a raw prefix but not as `<base>-`: outside.
        let lookalike = file(format!("{}x-Mail", f.bot), "Claude-Laptop".into()).await;

        let mut browse = search("*");
        browse.limit = Some(200);
        let r = handle_search_bus(&f.brain, browse, Visibility::Restricted(&family)).await;
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        let seen = ids(&v, "handoffs");
        for id in [f.to_bot, f.from_bot, member] {
            assert!(seen.contains(&id), "family member row {id} missing");
        }
        for id in [f.others, f.broadcast, lookalike] {
            assert!(
                !seen.contains(&id),
                "row {id} leaked past the family boundary"
            );
        }

        sqlx::query("DELETE FROM handoffs WHERE id = ANY($1)")
            .bind(vec![member, lookalike])
            .execute(&f.pool)
            .await
            .unwrap();
        cleanup(&f).await;
    }

    #[tokio::test]
    async fn by_id_handoff_tools_treat_foreign_rows_as_missing() {
        let f = fixture().await;
        let restricted = Visibility::Restricted(&f.bot);

        for id in [f.others, f.broadcast] {
            let r = handle_get_handoff(
                &f.brain,
                GetHandoffParams {
                    handoff_id: id.to_string(),
                },
                restricted,
            )
            .await;
            assert!(text(&r).contains("not found"), "{}", text(&r));

            let r = handle_accept_handoff(
                &f.brain,
                UpdateHandoffStatusParams {
                    handoff_id: id.to_string(),
                    agent_name: None,
                },
                None,
                restricted,
            )
            .await;
            assert!(text(&r).contains("not found"), "{}", text(&r));
        }
        // The refused accept wrote nothing.
        let others = ops_brain::repo::handoff_repo::get_handoff(&f.pool, f.others)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(others.status, "pending");

        // Its own handoff is served.
        let r = handle_get_handoff(
            &f.brain,
            GetHandoffParams {
                handoff_id: f.to_bot.to_string(),
            },
            restricted,
        )
        .await;
        assert_eq!(r.is_error, Some(false), "{}", text(&r));

        // No replying into a thread it can't see.
        let r = handle_create_handoff(
            &f.brain,
            CreateHandoffParams {
                from_agent: f.bot.clone(),
                to_agent: Some("Claude-Buildbox".to_string()),
                priority: None,
                category: Some("notify".to_string()),
                title: "reply".to_string(),
                body: "body".to_string(),
                context: None,
                in_reply_to: Some(f.others.to_string()),
                complete_parent: None,
                commit_hash: None,
            },
            Some(&f.bot),
            restricted,
        )
        .await;
        assert!(text(&r).contains("not found"), "{}", text(&r));

        cleanup(&f).await;
    }

    #[tokio::test]
    async fn by_id_writes_refuse_foreign_rows() {
        use ops_brain::tools::coordination::{
            handle_complete_handoff, handle_delete_handoff, CompleteHandoffParams,
            DeleteHandoffParams,
        };
        let f = fixture().await;
        let restricted = Visibility::Restricted(&f.bot);
        let id = f.others.to_string();

        let r = handle_complete_handoff(
            &f.brain,
            CompleteHandoffParams {
                handoff_id: id.clone(),
                commit_hash: Some("abc123".to_string()),
            },
            restricted,
        )
        .await;
        assert!(text(&r).contains("not found"), "{}", text(&r));
        let r = handle_delete_handoff(
            &f.brain,
            DeleteHandoffParams {
                handoff_id: id.clone(),
            },
            Some(&f.bot),
            restricted,
        )
        .await;
        assert!(text(&r).contains("not found"), "{}", text(&r));

        let row = ops_brain::repo::handoff_repo::get_handoff(&f.pool, f.others)
            .await
            .unwrap()
            .expect("foreign handoff survives");
        assert_eq!(row.status, "pending");
        assert!(row.commit_hash.is_none());

        // Its own handoff completes normally.
        let r = handle_complete_handoff(
            &f.brain,
            CompleteHandoffParams {
                handoff_id: f.to_bot.to_string(),
                commit_hash: None,
            },
            restricted,
        )
        .await;
        assert_eq!(r.is_error, Some(false), "{}", text(&r));

        cleanup(&f).await;
    }

    #[tokio::test]
    async fn queues_are_own_only_and_drop_broadcasts() {
        let f = fixture().await;
        let restricted = Visibility::Restricted(&f.bot);

        let r = handle_check_in(
            &f.brain,
            CheckInParams {
                agent_name: "Claude-Laptop".to_string(),
            },
            Some(&f.bot),
            restricted,
        )
        .await;
        assert_eq!(r.is_error, Some(true));
        assert!(text(&r).contains("may only query its own agent"));

        let r = handle_check_in(
            &f.brain,
            CheckInParams {
                agent_name: f.bot.clone(),
            },
            Some(&f.bot),
            restricted,
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        let listed: Vec<Uuid> = v["open_handoffs_to_you"]["items"]
            .as_array()
            .unwrap()
            .iter()
            .map(|i| i["id"].as_str().unwrap().parse().unwrap())
            .collect();
        assert!(listed.contains(&f.to_bot));
        assert!(!listed.contains(&f.broadcast), "broadcasts are outside");

        let r = handle_list_handoffs(
            &f.brain,
            ListHandoffsParams {
                status: None,
                to_agent: None,
                from_agent: None,
                category: None,
                include_notify: Some(true),
                limit: Some(200),
                compact: Some(true),
            },
            restricted,
        )
        .await;
        let v: serde_json::Value = serde_json::from_str(&text(&r)).unwrap();
        let listed = ids(&v, "handoffs");
        assert!(listed.contains(&f.to_bot) && listed.contains(&f.from_bot));
        for item in v["handoffs"].as_array().unwrap() {
            let from = item["from_agent"].as_str().unwrap();
            let to = item["to_agent"].as_str().unwrap_or("");
            assert!(
                from.eq_ignore_ascii_case(&f.bot) || to.eq_ignore_ascii_case(&f.bot),
                "foreign handoff listed: {item}"
            );
        }

        cleanup(&f).await;
    }
}
