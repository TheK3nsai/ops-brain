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

// ===== Session & Handoff Repo =====

mod coordination_tests {
    use super::*;

    #[tokio::test]
    async fn session_lifecycle() {
        let pool = pool().await;
        let machine_id = format!("test-{}", Uuid::now_v7());

        let session = ops_brain::repo::session_repo::start_session(&pool, &machine_id, "test-host")
            .await
            .unwrap();

        assert!(session.ended_at.is_none());
        assert_eq!(session.machine_hostname, "test-host");

        // List active sessions
        let active =
            ops_brain::repo::session_repo::list_sessions(&pool, Some(&machine_id), true, 10)
                .await
                .unwrap();
        assert!(active.iter().any(|s| s.id == session.id));

        // End session
        let ended = ops_brain::repo::session_repo::end_session(
            &pool,
            session.id,
            Some("Completed testing"),
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
        let pending =
            ops_brain::repo::handoff_repo::list_handoffs(&pool, Some("pending"), None, None, 10)
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
        assert_eq!(
            briefing.content,
            "# Daily Briefing\n\nAll systems operational."
        );

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

mod runbook_execution_tests {
    use super::*;

    #[tokio::test]
    async fn log_and_list_execution() {
        let pool = pool().await;
        let slug = format!("test-rb-exec-{}", Uuid::now_v7());

        // Create a runbook to reference
        let runbook = ops_brain::repo::runbook_repo::create_runbook(
            &pool,
            "Test Runbook for Execution",
            &slug,
            Some("testing"),
            "Step 1: do the thing",
            &[],
            Some(10),
            false,
            None,
            None,
            false,
        )
        .await
        .unwrap();

        // Log an execution
        let exec = ops_brain::repo::runbook_execution_repo::log_execution(
            &pool,
            runbook.id,
            "CC-Stealth",
            "success",
            Some("DR test completed, all systems restored"),
            Some(45),
            None,
        )
        .await
        .unwrap();

        assert_eq!(exec.runbook_id, runbook.id);
        assert_eq!(exec.executor, "CC-Stealth");
        assert_eq!(exec.result, "success");
        assert_eq!(
            exec.notes.as_deref(),
            Some("DR test completed, all systems restored")
        );
        assert_eq!(exec.duration_minutes, Some(45));

        // Log another execution
        let exec2 = ops_brain::repo::runbook_execution_repo::log_execution(
            &pool,
            runbook.id,
            "CC-HSR",
            "failure",
            Some("Network issue during step 3"),
            Some(15),
            None,
        )
        .await
        .unwrap();

        assert_eq!(exec2.result, "failure");

        // List executions for this runbook
        let list = ops_brain::repo::runbook_execution_repo::list_executions_for_runbook(
            &pool, runbook.id, 10,
        )
        .await
        .unwrap();

        assert_eq!(list.len(), 2);

        // List recent executions (global)
        let recent = ops_brain::repo::runbook_execution_repo::list_recent_executions(&pool, 100)
            .await
            .unwrap();
        assert!(recent.iter().any(|e| e.id == exec.id));
        assert!(recent.iter().any(|e| e.id == exec2.id));

        // Cleanup
        sqlx::query("DELETE FROM runbook_executions WHERE runbook_id = $1")
            .bind(runbook.id)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM runbooks WHERE id = $1")
            .bind(runbook.id)
            .execute(&pool)
            .await
            .unwrap();
    }
}
