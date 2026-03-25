//! Integration tests for ops-brain.
//!
//! Requires a running PostgreSQL instance. Uses DATABASE_URL from environment
//! or defaults to the test database. Each test runs in a transaction that gets
//! rolled back for isolation.
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
            &pool, "Test Client", &slug, Some("test notes"), None, None, None,
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

        let c1 = ops_brain::repo::client_repo::upsert_client(
            &pool, "Original", &slug, Some("v1"), None, None, None,
        )
        .await
        .unwrap();

        let c2 = ops_brain::repo::client_repo::upsert_client(
            &pool, "Updated", &slug, Some("v2"), Some(10), None, None,
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
        let clients = ops_brain::repo::client_repo::list_clients(&pool).await.unwrap();
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
            &pool, "Test Client", &client_slug, None, None, None, None,
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
        )
        .await
        .unwrap();

        assert_eq!(k.title, "Test Knowledge Entry");
        assert!(!k.cross_client_safe);

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
            &pool, "Filter Test Open", "high", None, None, None,
        )
        .await
        .unwrap();

        let i2 = ops_brain::repo::incident_repo::create_incident(
            &pool, "Filter Test Critical", "critical", None, None, None,
        )
        .await
        .unwrap();

        // List by severity
        let critical = ops_brain::repo::incident_repo::list_incidents(
            &pool, None, None, Some("critical"), 100,
        )
        .await
        .unwrap();
        assert!(critical.iter().any(|i| i.id == i2.id));

        // List by status
        let open = ops_brain::repo::incident_repo::list_incidents(
            &pool, None, Some("open"), None, 100,
        )
        .await
        .unwrap();
        assert!(open.iter().any(|i| i.id == i1.id));
        assert!(open.iter().any(|i| i.id == i2.id));

        // Cleanup
        sqlx::query("DELETE FROM incidents WHERE id = ANY($1)")
            .bind(&[i1.id, i2.id])
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Session & Handoff Repo =====

mod coordination_tests {
    use super::*;

    #[tokio::test]
    async fn session_lifecycle() {
        let pool = pool().await;
        let machine_id = format!("test-{}", Uuid::now_v7());

        let session = ops_brain::repo::session_repo::start_session(
            &pool, &machine_id, "test-host",
        )
        .await
        .unwrap();

        assert!(session.ended_at.is_none());
        assert_eq!(session.machine_hostname, "test-host");

        // List active sessions
        let active = ops_brain::repo::session_repo::list_sessions(
            &pool, Some(&machine_id), true, 10,
        )
        .await
        .unwrap();
        assert!(active.iter().any(|s| s.id == session.id));

        // End session
        let ended = ops_brain::repo::session_repo::end_session(
            &pool, session.id, Some("Completed testing"),
        )
        .await
        .unwrap();
        assert!(ended.ended_at.is_some());
        assert_eq!(ended.summary.as_deref(), Some("Completed testing"));

        // Cleanup
        sqlx::query("DELETE FROM sessions WHERE id = $1")
            .bind(session.id)
            .execute(&pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn handoff_lifecycle() {
        let pool = pool().await;

        let handoff = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            None,
            "stealth",
            Some("cloudlab"),
            "high",
            "Continue DNS migration",
            "Need to update remaining A records for HSR",
            None,
        )
        .await
        .unwrap();

        assert_eq!(handoff.status, "pending");
        assert_eq!(handoff.from_machine, "stealth");
        assert_eq!(handoff.to_machine.as_deref(), Some("cloudlab"));

        // Accept
        let accepted = ops_brain::repo::handoff_repo::update_handoff_status(
            &pool, handoff.id, "accepted",
        )
        .await
        .unwrap();
        assert_eq!(accepted.status, "accepted");

        // Complete
        let completed = ops_brain::repo::handoff_repo::update_handoff_status(
            &pool, handoff.id, "completed",
        )
        .await
        .unwrap();
        assert_eq!(completed.status, "completed");

        // List by status
        let pending = ops_brain::repo::handoff_repo::list_handoffs(
            &pool, Some("pending"), None, None, 10,
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
            &pool, "Requesting Client", &slug_req, None, None, None, None,
        )
        .await
        .unwrap();

        let own_client = ops_brain::repo::client_repo::upsert_client(
            &pool, "Owning Client", &slug_own, None, None, None, None,
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
            .bind(&[req_client.id, own_client.id])
            .execute(&pool)
            .await
            .unwrap();
    }
}

// ===== Briefing Repo =====

mod briefing_tests {
    use super::*;

    #[tokio::test]
    async fn insert_and_retrieve_briefing() {
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
        assert_eq!(briefing.content, "# Daily Briefing\n\nAll systems operational.");

        // Get by ID
        let fetched = ops_brain::repo::briefing_repo::get_briefing(&pool, briefing.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(fetched.content, briefing.content);

        // List briefings
        let list = ops_brain::repo::briefing_repo::list_briefings(&pool, Some("daily"), None, 10)
            .await
            .unwrap();
        assert!(list.iter().any(|b| b.id == briefing.id));

        // Cleanup
        sqlx::query("DELETE FROM briefings WHERE id = $1")
            .bind(briefing.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
