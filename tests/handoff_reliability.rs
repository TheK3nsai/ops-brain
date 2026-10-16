//! Retry and claim invariants against the explicitly isolated integration DB.
mod common;

use ops_brain::{
    auth::Visibility,
    repo::handoff_repo::{self as repo, CreateHandoffError, CreateHandoffOutcome},
    tools::{
        coordination::{
            handle_accept_handoff, handle_create_handoff, handle_get_handoff, CreateHandoffParams,
            GetHandoffParams, UpdateHandoffStatusParams,
        },
        OpsBrain,
    },
};
use sqlx::PgPool;
use uuid::Uuid;

async fn create(
    pool: &PgPool,
    sender: &str,
    key: &str,
    title: &str,
    context: Option<&serde_json::Value>,
    parent: Option<Uuid>,
) -> Result<CreateHandoffOutcome, CreateHandoffError> {
    repo::create_handoff_idempotent(
        pool,
        sender,
        None,
        "normal",
        "action",
        title,
        "retry body",
        context,
        parent,
        Some(key),
    )
    .await
}

fn response_text(response: &rmcp::model::CallToolResult) -> String {
    serde_json::to_string(response).unwrap()
}

#[tokio::test]
async fn replay_preserves_terminal_row_and_original_request_after_parent_deletion() {
    let pool = common::test_pool().await;
    let sender = format!("Retry-{}", Uuid::now_v7());
    let parent = repo::create_handoff(
        &pool, &sender, None, "normal", "action", "parent", "body", None, None,
    )
    .await
    .unwrap();
    let first = create(
        &pool,
        &sender,
        "request-1",
        "original",
        Some(&serde_json::json!({"a": 1, "b": {"x": 2}})),
        Some(parent.id),
    )
    .await
    .unwrap();
    assert!(first.created);
    let completed = repo::complete_handoff_with_commit(&pool, first.handoff.id, Some("abc123"))
        .await
        .unwrap()
        .unwrap();
    repo::delete_handoff(&pool, parent.id).await.unwrap();
    let persisted = repo::get_handoff(&pool, completed.id)
        .await
        .unwrap()
        .unwrap();
    let replay = create(
        &pool,
        &sender.to_ascii_lowercase(),
        "request-1",
        "original",
        Some(&serde_json::json!({"b": {"x": 2.0}, "a": 1})),
        Some(parent.id),
    )
    .await
    .unwrap();
    assert!(!replay.created);
    assert_eq!(replay.handoff.id, first.handoff.id);
    assert_eq!(replay.handoff.status, "completed");
    assert_eq!(replay.handoff.commit_hash.as_deref(), Some("abc123"));
    assert_eq!(replay.handoff.created_at, first.handoff.created_at);
    // Parent deletion legitimately updates the row; retry must not touch it.
    assert_eq!(replay.handoff.updated_at, persisted.updated_at);
    assert_eq!(replay.handoff.in_reply_to, None);
    assert_eq!(replay.handoff.repeat_count, 0);
    let brain = OpsBrain::new(pool.clone(), None);
    let params = |title: &str| CreateHandoffParams {
        from_agent: sender.clone(),
        to_agent: None,
        priority: None,
        category: None,
        title: title.into(),
        body: "retry body".into(),
        context: Some(serde_json::json!({"b": {"x": 2}, "a": 1})),
        in_reply_to: Some(parent.id.to_string()),
        idempotency_key: Some("request-1".into()),
    };
    let replay_response = handle_create_handoff(
        &brain,
        params("original"),
        Some(&sender),
        Visibility::Restricted(&sender),
    )
    .await;
    assert!(
        !replay_response.is_error.unwrap_or(false),
        "{}",
        response_text(&replay_response)
    );
    assert!(response_text(&replay_response).contains(&first.handoff.id.to_string()));
    let conflict_response = handle_create_handoff(
        &brain,
        params("private-changed-payload"),
        Some(&sender),
        Visibility::Restricted(&sender),
    )
    .await;
    assert!(conflict_response.is_error.unwrap_or(false));
    assert!(response_text(&conflict_response).contains("different request"));
    assert!(!response_text(&conflict_response).contains("private-changed-payload"));
    assert!(matches!(
        create(
            &pool,
            &sender,
            "request-1",
            "changed",
            None,
            Some(parent.id)
        )
        .await,
        Err(CreateHandoffError::IdempotencyConflict)
    ));
    let independent_sender = format!("Other-{}", Uuid::now_v7());
    let independent = create(
        &pool,
        &independent_sender,
        "request-1",
        "changed",
        None,
        None,
    )
    .await
    .unwrap();
    assert!(independent.created);
    assert_ne!(independent.handoff.id, first.handoff.id);
    for id in [first.handoff.id, independent.handoff.id] {
        repo::delete_handoff(&pool, id).await.unwrap();
    }
}

#[tokio::test]
async fn concurrent_retries_insert_once_and_conflicting_payloads_never_overwrite() {
    let pool = common::test_pool().await;
    let sender = format!("Concurrent-{}", Uuid::now_v7());
    let barrier = std::sync::Arc::new(tokio::sync::Barrier::new(12));
    let mut tasks = Vec::new();
    for _ in 0..12 {
        let (pool, sender, barrier) = (pool.clone(), sender.clone(), barrier.clone());
        tasks.push(tokio::spawn(async move {
            barrier.wait().await;
            create(&pool, &sender, "same", "original", None, None)
                .await
                .unwrap()
        }));
    }
    let mut outcomes = Vec::new();
    for task in tasks {
        outcomes.push(task.await.unwrap());
    }
    let id = outcomes[0].handoff.id;
    assert_eq!(outcomes.iter().filter(|outcome| outcome.created).count(), 1);
    assert!(outcomes.iter().all(|outcome| outcome.handoff.id == id));
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM handoffs WHERE from_agent = $1")
        .bind(&sender)
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
    let mut conflicts = Vec::new();
    for _ in 0..4 {
        let (pool, sender) = (pool.clone(), sender.clone());
        conflicts.push(tokio::spawn(async move {
            assert!(matches!(
                create(&pool, &sender, "same", "different", None, None).await,
                Err(CreateHandoffError::IdempotencyConflict)
            ));
        }));
    }
    for task in conflicts {
        task.await.unwrap();
    }
    assert_eq!(
        repo::get_handoff(&pool, id).await.unwrap().unwrap().title,
        "original"
    );
    repo::delete_handoff(&pool, id).await.unwrap();
}

#[tokio::test]
async fn racing_acceptors_keep_broadcast_only_in_winners_check_in_and_wake_queue() {
    let pool = common::test_pool().await;
    let sender = format!("Sender-{}", Uuid::now_v7());
    let first = format!("Claimant-A-{}", Uuid::now_v7());
    let second = format!("Claimant-B-{}", Uuid::now_v7());
    let row = create(&pool, &sender, "claim", "broadcast", None, None)
        .await
        .unwrap()
        .handoff;
    let (a, b) = tokio::join!(
        repo::accept_handoff_as(&pool, row.id, Some(&first)),
        repo::accept_handoff_as(&pool, row.id, Some(&second))
    );
    let (a, b) = (a.unwrap(), b.unwrap());
    assert_ne!(a.is_some(), b.is_some());
    let claimed = a.or(b).unwrap();
    let winner = claimed.accepted_by.as_deref().unwrap();
    let loser = if winner == first { &second } else { &first };
    let brief = repo::list_open_action_briefs(&pool, winner)
        .await
        .unwrap()
        .into_iter()
        .find(|item| item.id == row.id)
        .unwrap();
    assert!(brief.is_operator);
    assert_eq!(brief.accepted_by.as_deref(), Some(winner));
    assert!(brief.to_agent.is_none());
    for (agent, expected) in [(winner, true), (loser.as_str(), false)] {
        let operational =
            repo::list_open_handoffs(&pool, Some(agent), None, None, false, true, true, None, 200)
                .await
                .unwrap();
        assert_eq!(operational.iter().any(|h| h.id == row.id), expected);
        let wake = repo::list_pending_for_agent(&pool, agent, None, 200)
            .await
            .unwrap();
        assert_eq!(wake.iter().any(|h| h.id == row.id), expected);
    }
    // Claimant attribution is provenance, not a new restricted-token grant.
    let restricted = repo::list_open_handoffs(
        &pool,
        Some(winner),
        None,
        None,
        false,
        true,
        true,
        Some(winner),
        200,
    )
    .await
    .unwrap();
    assert!(!restricted.iter().any(|h| h.id == row.id));
    let brain = OpsBrain::new(pool.clone(), None);
    let hidden = handle_get_handoff(
        &brain,
        GetHandoffParams {
            handoff_id: row.id.to_string(),
        },
        Visibility::Restricted(winner),
    )
    .await;
    assert!(response_text(&hidden).contains("not found"));
    repo::delete_handoff(&pool, row.id).await.unwrap();
}

#[tokio::test]
async fn accept_handler_binds_identity_and_keeps_restricted_boundary() {
    let pool = common::test_pool().await;
    let sender = format!("Sender-{}", Uuid::now_v7());
    let agent = format!("Bound-{}", Uuid::now_v7());
    let row = create(&pool, &sender, "bound", "broadcast", None, None)
        .await
        .unwrap()
        .handoff;
    let brain = OpsBrain::new(pool.clone(), None);
    let params = |name: Option<String>| UpdateHandoffStatusParams {
        handoff_id: row.id.to_string(),
        agent_name: name,
    };
    let no_identity = handle_accept_handoff(&brain, params(None), None, Visibility::Full).await;
    assert!(no_identity.is_error.unwrap_or(false));
    let denied = handle_accept_handoff(
        &brain,
        params(None),
        Some(&agent),
        Visibility::Restricted(&agent),
    )
    .await;
    assert!(response_text(&denied).contains("not found"));
    let mismatch = handle_accept_handoff(
        &brain,
        params(Some("Impersonated".into())),
        Some(&agent),
        Visibility::Full,
    )
    .await;
    assert!(mismatch.is_error.unwrap_or(false));
    assert_eq!(
        repo::get_handoff(&pool, row.id)
            .await
            .unwrap()
            .unwrap()
            .status,
        "pending"
    );
    let accepted =
        handle_accept_handoff(&brain, params(None), Some(&agent), Visibility::Full).await;
    assert!(
        !accepted.is_error.unwrap_or(false),
        "{}",
        response_text(&accepted)
    );
    assert_eq!(
        repo::get_handoff(&pool, row.id)
            .await
            .unwrap()
            .unwrap()
            .accepted_by
            .as_deref(),
        Some(agent.as_str())
    );
    repo::delete_handoff(&pool, row.id).await.unwrap();
}

#[tokio::test]
async fn family_accepts_require_explicit_concrete_identity() {
    let pool = common::test_pool().await;
    let sender = format!("Sender-{}", Uuid::now_v7());
    let base = format!("Family-{}", Uuid::now_v7());
    let binding = format!("{base}-*");
    let member = format!("{base}-Coder");
    let brain = OpsBrain::new(pool.clone(), None);

    // Both the base and a suffix member are valid concrete family identities.
    // Explicit identity is required even when the recipient could be inferred.
    for (identity, recipient) in [
        (base.as_str(), None),
        (member.as_str(), Some(member.as_str())),
    ] {
        let row = repo::create_handoff(
            &pool,
            &sender,
            recipient,
            "normal",
            "action",
            "family ownership",
            "body",
            None,
            None,
        )
        .await
        .unwrap();
        let params = |agent_name: Option<String>| UpdateHandoffStatusParams {
            handoff_id: row.id.to_string(),
            agent_name,
        };
        for supplied in [
            None,
            Some("Outside-Family".to_string()),
            Some(binding.clone()),
        ] {
            let missing = supplied.is_none();
            let refused =
                handle_accept_handoff(&brain, params(supplied), Some(&binding), Visibility::Full)
                    .await;
            assert!(
                refused.is_error.unwrap_or(false),
                "{}",
                response_text(&refused)
            );
            if missing {
                let text = response_text(&refused);
                assert!(
                    text.contains(&format!("family token {binding} must pass agent_name"))
                        && text.contains(&format!("{base}-<Bot>")),
                    "{text}"
                );
            }
            let unchanged = repo::get_handoff(&pool, row.id).await.unwrap().unwrap();
            assert_eq!(unchanged.status, "pending");
            assert_eq!(unchanged.accepted_by, None);
        }
        let accepted = handle_accept_handoff(
            &brain,
            params(Some(identity.to_string())),
            Some(&binding),
            Visibility::Full,
        )
        .await;
        assert!(
            !accepted.is_error.unwrap_or(false),
            "{}",
            response_text(&accepted)
        );
        let claimed = repo::get_handoff(&pool, row.id).await.unwrap().unwrap();
        assert_eq!(claimed.accepted_by.as_deref(), Some(identity));
        assert!(!claimed.accepted_by.unwrap().contains('*'));
        repo::delete_handoff(&pool, row.id).await.unwrap();
    }
}
