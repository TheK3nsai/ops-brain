use rmcp::model::CallToolResult;
use schemars::JsonSchema;
use serde::Deserialize;
use uuid::Uuid;

use super::helpers::{error_result, json_result};

#[derive(Debug, Default, Deserialize, JsonSchema)]
pub struct ListLivePeersParams {}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendLiveMessageParams {
    /// Target peer ID from list_live_peers. Your token-bound agent must have
    /// exactly one connected local adapter for unambiguous source provenance.
    pub to_peer_id: String,
    /// Untrusted peer-to-peer text (1–8,000 bytes). Never send secrets, PII,
    /// PHI, credentials, or file contents.
    pub message: String,
    /// Optional live message UUID being replied to.
    pub in_reply_to: Option<String>,
    /// UUID idempotency key generated once per intended message and reused on retry.
    pub idempotency_key: String,
}

pub async fn handle_list_live_peers(
    brain: &super::OpsBrain,
    bound: Option<&str>,
) -> CallToolResult {
    let Some(bound) = bound else {
        return error_result(
            "live peer tools require an identity-bound per-agent token; the main bearer and \
             stdio transport cannot establish live provenance",
        );
    };
    let peers = brain.live_hub.list().await;
    json_result(&serde_json::json!({
        "requesting_agent": bound,
        "count": peers.len(),
        "peers": peers,
        "delivery": "online-only; use create_handoff when a peer is absent",
    }))
}

pub async fn handle_send_live_message(
    brain: &super::OpsBrain,
    p: SendLiveMessageParams,
    bound: Option<&str>,
) -> CallToolResult {
    let Some(bound) = bound else {
        return error_result(
            "live peer tools require an identity-bound per-agent token; the main bearer and \
             stdio transport cannot establish live provenance",
        );
    };
    let to_peer_id = match Uuid::parse_str(&p.to_peer_id) {
        Ok(id) => id,
        Err(_) => return error_result("to_peer_id must be a full UUID"),
    };
    let in_reply_to = match p.in_reply_to.as_deref() {
        Some(value) => match Uuid::parse_str(value) {
            Ok(id) => Some(id),
            Err(_) => return error_result("in_reply_to must be a full UUID when present"),
        },
        None => None,
    };
    let idempotency_key = match Uuid::parse_str(&p.idempotency_key) {
        Ok(id) => id,
        Err(_) => return error_result("idempotency_key must be a full UUID"),
    };

    match brain
        .live_hub
        .send_from_agent(bound, to_peer_id, &p.message, in_reply_to, idempotency_key)
        .await
    {
        Ok(receipt) => json_result(&receipt),
        Err(error) => error_result(&error.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::live::{LiveAdapter, LiveHub};
    use sqlx::postgres::PgPoolOptions;

    fn brain(hub: LiveHub) -> super::super::OpsBrain {
        let pool = PgPoolOptions::new()
            .connect_lazy("postgresql://unused:unused@127.0.0.1:1/unused")
            .unwrap();
        super::super::OpsBrain::with_live_hub(pool, None, hub)
    }

    #[tokio::test]
    async fn unbound_callers_cannot_use_live_tools() {
        let result = handle_list_live_peers(&brain(LiveHub::default()), None).await;
        assert_eq!(result.is_error, Some(true));
    }

    #[tokio::test]
    async fn bound_list_uses_shared_hub() {
        let hub = LiveHub::default();
        hub.register("Codex-Stealth", LiveAdapter::Codex, "codex-1")
            .await
            .unwrap();
        let result = handle_list_live_peers(&brain(hub), Some("Codex-Stealth")).await;
        assert_eq!(result.is_error, Some(false));
        assert_eq!(
            result
                .structured_content
                .as_ref()
                .and_then(|value| value.get("count"))
                .and_then(serde_json::Value::as_u64),
            Some(1)
        );
    }
}
