//! Claim invariants against the explicitly isolated integration DB.
mod common;

use ops_brain::{
    auth::Visibility,
    models::handoff::Handoff,
    repo::handoff_repo as repo,
    tools::{
        coordination::{
            handle_accept_handoff, handle_get_handoff, GetHandoffParams, UpdateHandoffStatusParams,
        },
        OpsBrain,
    },
};
use sqlx::PgPool;
use uuid::Uuid;

async fn create(pool: &PgPool, sender: &str, title: &str) -> Handoff {
    repo::create_handoff(
        pool, sender, None, "normal", "action", title, "body", None, None,
    )
    .await
    .unwrap()
}

fn response_text(response: &rmcp::model::CallToolResult) -> String {
    serde_json::to_string(response).unwrap()
}

#[tokio::test]
async fn racing_acceptors_keep_broadcast_only_in_winners_check_in_and_wake_queue() {
    let pool = common::test_pool().await;
    let sender = format!("Sender-{}", Uuid::now_v7());
    let first = format!("Claimant-A-{}", Uuid::now_v7());
    let second = format!("Claimant-B-{}", Uuid::now_v7());
    let row = create(&pool, &sender, "broadcast").await;
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
    let row = create(&pool, &sender, "broadcast").await;
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
