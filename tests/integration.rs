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
    async fn handoff_lifecycle() {
        let pool = pool().await;

        let handoff = ops_brain::repo::handoff_repo::create_handoff(
            &pool,
            None,
            "dev-laptop",
            Some("prod-server"),
            "high",
            "Continue DNS migration",
            "Need to update remaining A records",
            None,
        )
        .await
        .unwrap();

        assert_eq!(handoff.status, "pending");
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

// ===== CC Identity Repo & Team Flow =====

mod cc_team_tests {
    use super::*;

    /// Use a unique test cc_name (not a real one) so we never collide with
    /// real entries on a shared dev DB. The repo accepts any TEXT key — the
    /// CC_TEAM allowlist is enforced one layer up at the handler.
    fn test_cc_name() -> String {
        format!("TEST-CC-{}", Uuid::now_v7())
    }

    async fn delete_identity(pool: &PgPool, cc_name: &str) {
        sqlx::query("DELETE FROM cc_identities WHERE cc_name = $1")
            .bind(cc_name)
            .execute(pool)
            .await
            .unwrap();
    }

    #[tokio::test]
    async fn upsert_first_write_returns_first_write_true() {
        let pool = pool().await;
        let cc = test_cc_name();

        let (row, was_first) = ops_brain::repo::cc_identity_repo::upsert(
            &pool,
            &cc,
            "I am a test identity for the cc team flow.",
        )
        .await
        .unwrap();

        assert_eq!(row.cc_name, cc);
        assert!(row.body.contains("test identity"));
        assert!(was_first, "first write should report was_first=true");

        delete_identity(&pool, &cc).await;
    }

    #[tokio::test]
    async fn upsert_second_write_reports_not_first() {
        let pool = pool().await;
        let cc = test_cc_name();

        let (_, first) =
            ops_brain::repo::cc_identity_repo::upsert(&pool, &cc, "first body content here")
                .await
                .unwrap();
        assert!(first);

        let (row, second) = ops_brain::repo::cc_identity_repo::upsert(
            &pool,
            &cc,
            "second body content overwrites first",
        )
        .await
        .unwrap();
        assert!(!second, "subsequent write should report was_first=false");
        assert!(row.body.contains("second body"));
        assert!(!row.body.contains("first body"));

        delete_identity(&pool, &cc).await;
    }

    #[tokio::test]
    async fn get_returns_none_for_unknown_cc() {
        let pool = pool().await;
        let result =
            ops_brain::repo::cc_identity_repo::get(&pool, "TEST-CC-definitely-not-real-12345")
                .await
                .unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn list_all_includes_recent_upsert() {
        let pool = pool().await;
        let cc = test_cc_name();

        ops_brain::repo::cc_identity_repo::upsert(&pool, &cc, "list-all visibility check body")
            .await
            .unwrap();

        let all = ops_brain::repo::cc_identity_repo::list_all(&pool)
            .await
            .unwrap();
        assert!(
            all.iter().any(|i| i.cc_name == cc),
            "list_all should include the just-upserted row"
        );

        delete_identity(&pool, &cc).await;
    }

    #[tokio::test]
    async fn upsert_updates_timestamp() {
        let pool = pool().await;
        let cc = test_cc_name();

        let (r1, _) =
            ops_brain::repo::cc_identity_repo::upsert(&pool, &cc, "original timestamp body")
                .await
                .unwrap();
        let t1 = r1.updated_at;

        // Sleep just enough for NOW() to advance.
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;

        let (r2, _) =
            ops_brain::repo::cc_identity_repo::upsert(&pool, &cc, "rewrite timestamp body")
                .await
                .unwrap();
        assert!(r2.updated_at > t1, "updated_at should advance on rewrite");

        delete_identity(&pool, &cc).await;
    }

    // ===== Handler-level safety guards =====
    //
    // The repo tests above cover the data layer. These tests cover the
    // handler guards that make the "a CC can only ever update its own row"
    // claim provably true: invalid name rejection, set_my_identity requires
    // prior check_in, and the no-mid-session-impersonation rule.

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
    async fn handler_set_my_identity_requires_prior_check_in() {
        let brain = build_brain(pool().await);
        // No check_in called — this is the safety guarantee under test.
        let result = ops_brain::tools::cc_team::handle_set_my_identity(
            &brain,
            ops_brain::tools::cc_team::SetMyIdentityParams {
                body: "I am a confident teammate with a clear scope to defend.".to_string(),
            },
        )
        .await;
        assert_eq!(result.is_error, Some(true));
        let text = extract_text(&result);
        assert!(text.contains("haven't checked in"));
    }

    #[tokio::test]
    async fn handler_check_in_rejects_conflicting_name_swap() {
        let brain = build_brain(pool().await);

        let r1 = ops_brain::tools::cc_team::handle_check_in(
            &brain,
            ops_brain::tools::cc_team::CheckInParams {
                my_name: "CC-Stealth".to_string(),
            },
        )
        .await;
        assert_eq!(
            r1.is_error,
            Some(false),
            "first valid check_in should succeed"
        );

        // Same session attempting to switch identity must be rejected.
        let r2 = ops_brain::tools::cc_team::handle_check_in(
            &brain,
            ops_brain::tools::cc_team::CheckInParams {
                my_name: "CC-CPA".to_string(),
            },
        )
        .await;
        assert_eq!(r2.is_error, Some(true));
        let text = extract_text(&r2);
        assert!(text.contains("Already checked in"));
        assert!(text.contains("CC-Stealth"));
        assert!(text.contains("CC-CPA"));
    }

    #[tokio::test]
    async fn handler_check_in_same_name_is_idempotent() {
        let brain = build_brain(pool().await);

        let r1 = ops_brain::tools::cc_team::handle_check_in(
            &brain,
            ops_brain::tools::cc_team::CheckInParams {
                my_name: "CC-Stealth".to_string(),
            },
        )
        .await;
        assert_eq!(r1.is_error, Some(false));

        // Same name → refresh, should succeed again.
        let r2 = ops_brain::tools::cc_team::handle_check_in(
            &brain,
            ops_brain::tools::cc_team::CheckInParams {
                my_name: "CC-Stealth".to_string(),
            },
        )
        .await;
        assert_eq!(
            r2.is_error,
            Some(false),
            "same-name re-check should succeed (refresh ritual)"
        );
    }
}
