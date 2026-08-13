//! Ephemeral live-peer transport.
//!
//! This module deliberately does not model durable sessions. A peer exists
//! only while its authenticated WebSocket is connected; disconnecting removes
//! it immediately, and undeliverable messages are never queued. Durable or
//! offline coordination remains the handoff subsystem's job.

use std::{
    collections::{HashMap, VecDeque},
    str::FromStr,
    sync::Arc,
    time::{Duration, Instant},
};

use axum::{
    extract::{ws::Message, ws::WebSocket, WebSocketUpgrade},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, oneshot, Mutex, Semaphore};
use uuid::Uuid;

use crate::{auth::CallerClass, validation::validate_bounded_text};

pub const LIVE_PROTOCOL_VERSION: u8 = 1;
const MAX_LABEL_BYTES: usize = 80;
const MAX_MESSAGE_BYTES: usize = 8_000;
const PEER_QUEUE_CAPACITY: usize = 32;
const SENDS_PER_MINUTE: usize = 30;
const IDEMPOTENCY_WINDOW: Duration = Duration::from_secs(600);
const MAX_RECENT_SENDS: usize = 4_096;
const ACK_TIMEOUT: Duration = Duration::from_secs(3);
const REGISTER_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PEERS: usize = 256;
const MAX_PEERS_PER_AGENT: usize = 8;
const MAX_WIRE_MESSAGE_BYTES: usize = 16 * 1024;
const MAX_IN_FLIGHT_SENDS_PER_PEER: usize = 8;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum LiveAdapter {
    ClaudeCode,
    Codex,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LivePeer {
    pub peer_id: Uuid,
    pub agent_name: String,
    pub adapter: LiveAdapter,
    pub label: String,
    pub metadata_trust: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct LiveMessage {
    pub message_id: Uuid,
    pub reply_peer_id: Uuid,
    pub from_agent: String,
    pub body: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub in_reply_to: Option<Uuid>,
    pub trust: &'static str,
    pub source_binding: &'static str,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DeliveryStatus {
    HostAccepted,
    Routed,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct DeliveryReceipt {
    pub message_id: Uuid,
    pub status: DeliveryStatus,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LiveError {
    Invalid(String),
    Unauthorized(String),
    NotFound(String),
    Busy(String),
    RateLimited(String),
    Duplicate(String),
    Rejected(String),
}

impl std::fmt::Display for LiveError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let message = match self {
            Self::Invalid(message)
            | Self::Unauthorized(message)
            | Self::NotFound(message)
            | Self::Busy(message)
            | Self::RateLimited(message)
            | Self::Duplicate(message)
            | Self::Rejected(message) => message,
        };
        write!(f, "{message}")
    }
}

#[derive(Debug, Clone)]
enum HostAck {
    Accepted,
    Rejected(String),
}

#[derive(Debug)]
struct PeerEntry {
    info: LivePeer,
    tx: mpsc::Sender<LiveMessage>,
}

#[derive(Debug)]
struct RecentSend {
    from_agent: String,
    idempotency_key: Uuid,
    sent_at: Instant,
}

struct SendSpec<'a> {
    bound_agent: &'a str,
    from_peer_id: Uuid,
    to_peer_id: Uuid,
    body: &'a str,
    in_reply_to: Option<Uuid>,
    idempotency_key: Uuid,
    source_binding: &'static str,
}

#[derive(Debug, Default)]
struct HubState {
    peers: HashMap<Uuid, PeerEntry>,
    pending_acks: HashMap<(Uuid, Uuid), oneshot::Sender<HostAck>>,
    sends: HashMap<Uuid, VecDeque<Instant>>,
    recent: VecDeque<RecentSend>,
}

#[derive(Debug, Clone, Default)]
pub struct LiveHub {
    state: Arc<Mutex<HubState>>,
}

pub struct LiveRegistration {
    pub peer: LivePeer,
    pub receiver: mpsc::Receiver<LiveMessage>,
}

impl LiveHub {
    pub async fn register(
        &self,
        agent_name: &str,
        adapter: LiveAdapter,
        label: &str,
    ) -> Result<LiveRegistration, LiveError> {
        crate::validation::validate_agent_name(agent_name).map_err(LiveError::Invalid)?;
        validate_live_label(label)?;

        let peer = LivePeer {
            peer_id: Uuid::now_v7(),
            agent_name: agent_name.to_string(),
            adapter,
            label: label.trim().to_string(),
            metadata_trust: "self_reported",
        };
        let (tx, receiver) = mpsc::channel(PEER_QUEUE_CAPACITY);
        let mut state = self.state.lock().await;
        if state.peers.len() >= MAX_PEERS {
            return Err(LiveError::Busy(format!(
                "live peer capacity reached ({MAX_PEERS})"
            )));
        }
        let agent_peer_count = state
            .peers
            .values()
            .filter(|entry| entry.info.agent_name.eq_ignore_ascii_case(agent_name))
            .count();
        if agent_peer_count >= MAX_PEERS_PER_AGENT {
            return Err(LiveError::Busy(format!(
                "live peer limit reached for this agent ({MAX_PEERS_PER_AGENT})"
            )));
        }
        state.peers.insert(
            peer.peer_id,
            PeerEntry {
                info: peer.clone(),
                tx,
            },
        );
        drop(state);
        tracing::info!(
            peer_id = %peer.peer_id,
            agent_name = %peer.agent_name,
            adapter = ?peer.adapter,
            "live peer connected"
        );
        Ok(LiveRegistration { peer, receiver })
    }

    pub async fn unregister(&self, peer_id: Uuid) {
        let mut state = self.state.lock().await;
        let Some(peer) = state.peers.remove(&peer_id) else {
            return;
        };
        state.sends.remove(&peer_id);
        let pending_keys: Vec<(Uuid, Uuid)> = state
            .pending_acks
            .keys()
            .filter(|(target, _)| *target == peer_id)
            .copied()
            .collect();
        for key in pending_keys {
            if let Some(waiter) = state.pending_acks.remove(&key) {
                let _ = waiter.send(HostAck::Rejected("target peer disconnected".to_string()));
            }
        }
        tracing::info!(
            peer_id = %peer_id,
            agent_name = %peer.info.agent_name,
            "live peer disconnected"
        );
    }

    pub async fn list(&self) -> Vec<LivePeer> {
        let mut peers: Vec<LivePeer> = self
            .state
            .lock()
            .await
            .peers
            .values()
            .map(|peer| peer.info.clone())
            .collect();
        peers.sort_by(|a, b| {
            a.agent_name
                .cmp(&b.agent_name)
                .then(a.label.cmp(&b.label))
                .then(a.peer_id.cmp(&b.peer_id))
        });
        peers
    }

    pub async fn send(
        &self,
        bound_agent: &str,
        from_peer_id: Uuid,
        to_peer_id: Uuid,
        body: &str,
        in_reply_to: Option<Uuid>,
    ) -> Result<DeliveryReceipt, LiveError> {
        self.send_idempotent(SendSpec {
            bound_agent,
            from_peer_id,
            to_peer_id,
            body,
            in_reply_to,
            idempotency_key: Uuid::now_v7(),
            source_binding: "connection_bound",
        })
        .await
    }

    pub async fn send_from_agent(
        &self,
        bound_agent: &str,
        to_peer_id: Uuid,
        body: &str,
        in_reply_to: Option<Uuid>,
        idempotency_key: Uuid,
    ) -> Result<DeliveryReceipt, LiveError> {
        let from_peer_id = {
            let state = self.state.lock().await;
            let mut matches = state
                .peers
                .values()
                .filter(|entry| entry.info.agent_name.eq_ignore_ascii_case(bound_agent))
                .map(|entry| entry.info.peer_id);
            let Some(peer_id) = matches.next() else {
                return Err(LiveError::NotFound(
                    "your live adapter is not connected".to_string(),
                ));
            };
            if matches.next().is_some() {
                return Err(LiveError::Invalid(
                    "multiple live adapters share your agent identity; send through the local adapter so the source connection is unambiguous"
                        .to_string(),
                ));
            }
            peer_id
        };
        self.send_idempotent(SendSpec {
            bound_agent,
            from_peer_id,
            to_peer_id,
            body,
            in_reply_to,
            idempotency_key,
            source_binding: "agent_bound_unique_adapter",
        })
        .await
    }

    async fn send_idempotent(&self, spec: SendSpec<'_>) -> Result<DeliveryReceipt, LiveError> {
        let SendSpec {
            bound_agent,
            from_peer_id,
            to_peer_id,
            body,
            in_reply_to,
            idempotency_key,
            source_binding,
        } = spec;
        validate_bounded_text(body, "message", MAX_MESSAGE_BYTES).map_err(LiveError::Invalid)?;
        if from_peer_id == to_peer_id {
            return Err(LiveError::Invalid(
                "a live peer cannot message itself".to_string(),
            ));
        }

        let now = Instant::now();
        let (message, ack_receiver) = {
            let mut state = self.state.lock().await;
            let sender = state.peers.get(&from_peer_id).ok_or_else(|| {
                LiveError::NotFound(
                    "sender peer is not connected; reconnect the local adapter".to_string(),
                )
            })?;
            if !sender.info.agent_name.eq_ignore_ascii_case(bound_agent) {
                return Err(LiveError::Unauthorized(format!(
                    "sender peer belongs to '{}', not your token-bound agent '{}'",
                    sender.info.agent_name, bound_agent
                )));
            }
            if source_binding == "agent_bound_unique_adapter"
                && state
                    .peers
                    .values()
                    .filter(|entry| entry.info.agent_name.eq_ignore_ascii_case(bound_agent))
                    .count()
                    != 1
            {
                return Err(LiveError::Invalid(
                    "multiple live adapters share your agent identity; send through the local adapter so the source connection is unambiguous"
                        .to_string(),
                ));
            }
            let from_agent = sender.info.agent_name.clone();
            let target_tx = state
                .peers
                .get(&to_peer_id)
                .map(|target| target.tx.clone())
                .ok_or_else(|| {
                    LiveError::NotFound(
                        "target peer is offline; create a handoff for durable delivery".to_string(),
                    )
                })?;

            while state
                .recent
                .front()
                .is_some_and(|send| now.duration_since(send.sent_at) >= IDEMPOTENCY_WINDOW)
            {
                state.recent.pop_front();
            }
            if state.recent.iter().any(|send| {
                send.from_agent.eq_ignore_ascii_case(bound_agent)
                    && send.idempotency_key == idempotency_key
            }) {
                return Err(LiveError::Duplicate(
                    "duplicate live message idempotency key suppressed".to_string(),
                ));
            }

            {
                let send_times = state.sends.entry(from_peer_id).or_default();
                while send_times
                    .front()
                    .is_some_and(|sent| now.duration_since(*sent) >= Duration::from_secs(60))
                {
                    send_times.pop_front();
                }
                if send_times.len() >= SENDS_PER_MINUTE {
                    return Err(LiveError::RateLimited(format!(
                        "live send limit reached ({SENDS_PER_MINUTE}/minute)"
                    )));
                }
                send_times.push_back(now);
            }

            let message = LiveMessage {
                message_id: Uuid::now_v7(),
                reply_peer_id: from_peer_id,
                from_agent,
                body: body.to_string(),
                in_reply_to,
                trust: "untrusted_peer_input",
                source_binding,
            };
            let (ack_sender, ack_receiver) = oneshot::channel();
            state
                .pending_acks
                .insert((to_peer_id, message.message_id), ack_sender);

            if let Err(error) = target_tx.try_send(message.clone()) {
                state.pending_acks.remove(&(to_peer_id, message.message_id));
                return Err(match error {
                    mpsc::error::TrySendError::Full(_) => LiveError::Busy(
                        "target peer queue is full; retry later or create a handoff".to_string(),
                    ),
                    mpsc::error::TrySendError::Closed(_) => LiveError::NotFound(
                        "target peer disconnected; create a handoff for durable delivery"
                            .to_string(),
                    ),
                });
            }

            if state.recent.len() >= MAX_RECENT_SENDS {
                state.recent.pop_front();
            }
            state.recent.push_back(RecentSend {
                from_agent: bound_agent.to_string(),
                idempotency_key,
                sent_at: now,
            });
            (message, ack_receiver)
        };

        match tokio::time::timeout(ACK_TIMEOUT, ack_receiver).await {
            Ok(Ok(HostAck::Accepted)) => Ok(DeliveryReceipt {
                message_id: message.message_id,
                status: DeliveryStatus::HostAccepted,
                detail: "target adapter accepted the message for host injection".to_string(),
            }),
            Ok(Ok(HostAck::Rejected(reason))) => Err(LiveError::Rejected(reason)),
            Ok(Err(_)) | Err(_) => {
                self.state
                    .lock()
                    .await
                    .pending_acks
                    .remove(&(to_peer_id, message.message_id));
                Ok(DeliveryReceipt {
                    message_id: message.message_id,
                    status: DeliveryStatus::Routed,
                    detail:
                        "enqueued for the target connection; host acceptance was not acknowledged"
                            .to_string(),
                })
            }
        }
    }

    pub async fn acknowledge(
        &self,
        peer_id: Uuid,
        message_id: Uuid,
        accepted: bool,
    ) -> Result<(), LiveError> {
        let waiter = self
            .state
            .lock()
            .await
            .pending_acks
            .remove(&(peer_id, message_id))
            .ok_or_else(|| {
                LiveError::NotFound("message is not awaiting acknowledgement".to_string())
            })?;
        let ack = if accepted {
            HostAck::Accepted
        } else {
            HostAck::Rejected("target adapter rejected the message".to_string())
        };
        let _ = waiter.send(ack);
        Ok(())
    }
}

fn validate_live_label(label: &str) -> Result<(), LiveError> {
    validate_bounded_text(label, "label", MAX_LABEL_BYTES).map_err(LiveError::Invalid)?;
    let trimmed = label.trim();
    if !trimmed
        .chars()
        .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.'))
    {
        return Err(LiveError::Invalid(
            "label may contain only ASCII letters, digits, '.', '_' and '-'".to_string(),
        ));
    }
    Ok(())
}

#[derive(Debug, Clone)]
pub struct LiveEndpointConfig {
    pub hub: LiveHub,
    pub allowed_hosts: Arc<Vec<String>>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
enum ClientFrame {
    Register {
        protocol: u8,
        adapter: LiveAdapter,
        label: String,
    },
    Acknowledge {
        message_id: Uuid,
        accepted: bool,
    },
    ListPeers {
        request_id: Uuid,
    },
    SendMessage {
        request_id: Uuid,
        to_peer_id: Uuid,
        body: String,
        in_reply_to: Option<Uuid>,
    },
}

#[derive(Debug, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
enum ServerFrame {
    Registered {
        protocol_version: u8,
        peer: LivePeer,
    },
    Peers {
        request_id: Uuid,
        peers: Vec<LivePeer>,
    },
    SendResult {
        request_id: Uuid,
        receipt: DeliveryReceipt,
    },
    Message {
        message: LiveMessage,
    },
    Error {
        request_id: Option<Uuid>,
        code: &'static str,
        message: String,
    },
}

impl ServerFrame {
    fn error(request_id: Option<Uuid>, error: LiveError) -> Self {
        let code = match &error {
            LiveError::Invalid(_) => "invalid_request",
            LiveError::Unauthorized(_) => "unauthorized",
            LiveError::NotFound(_) => "not_found",
            LiveError::Busy(_) => "busy",
            LiveError::RateLimited(_) => "rate_limited",
            LiveError::Duplicate(_) => "duplicate",
            LiveError::Rejected(_) => "rejected",
        };
        Self::Error {
            request_id,
            code,
            message: error.to_string(),
        }
    }
}

pub async fn live_websocket(
    ws: WebSocketUpgrade,
    headers: HeaderMap,
    Extension(caller): Extension<CallerClass>,
    Extension(config): Extension<LiveEndpointConfig>,
) -> Result<Response, StatusCode> {
    let agent_name = live_agent_name(caller)?;
    if headers.contains_key("origin") {
        return Err(StatusCode::FORBIDDEN);
    }
    let host = headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .ok_or(StatusCode::BAD_REQUEST)?;
    if !host_allowed(host, &config.allowed_hosts) {
        return Err(StatusCode::FORBIDDEN);
    }

    Ok(ws
        .max_message_size(MAX_WIRE_MESSAGE_BYTES)
        .max_frame_size(MAX_WIRE_MESSAGE_BYTES)
        .on_upgrade(move |socket| serve_socket(socket, config.hub, agent_name))
        .into_response())
}

fn live_agent_name(caller: CallerClass) -> Result<String, StatusCode> {
    match caller {
        CallerClass::Agent(token) => Ok(token.from_agent.clone()),
        CallerClass::Full | CallerClass::Machine(_) => Err(StatusCode::FORBIDDEN),
    }
}

fn host_allowed(host: &str, allowed_hosts: &[String]) -> bool {
    let Ok(presented) = axum::http::uri::Authority::from_str(host) else {
        return false;
    };
    allowed_hosts.iter().any(|allowed| {
        let Ok(expected) = axum::http::uri::Authority::from_str(allowed) else {
            return false;
        };
        expected.host().eq_ignore_ascii_case(presented.host())
            && expected
                .port_u16()
                .is_none_or(|port| presented.port_u16() == Some(port))
    })
}

async fn serve_socket(mut socket: WebSocket, hub: LiveHub, agent_name: String) {
    let registration = match tokio::time::timeout(REGISTER_TIMEOUT, socket.recv()).await {
        Ok(Some(Ok(Message::Text(text)))) => match serde_json::from_str::<ClientFrame>(&text) {
            Ok(ClientFrame::Register {
                protocol,
                adapter,
                label,
            }) if protocol == LIVE_PROTOCOL_VERSION => {
                match hub.register(&agent_name, adapter, &label).await {
                    Ok(registration) => registration,
                    Err(error) => {
                        send_frame(&mut socket, ServerFrame::error(None, error)).await;
                        return;
                    }
                }
            }
            Ok(ClientFrame::Register { .. }) => {
                send_frame(
                    &mut socket,
                    ServerFrame::Error {
                        request_id: None,
                        code: "unsupported_protocol",
                        message: format!("protocol must be {LIVE_PROTOCOL_VERSION}"),
                    },
                )
                .await;
                return;
            }
            Ok(_) => {
                send_frame(
                    &mut socket,
                    ServerFrame::Error {
                        request_id: None,
                        code: "register_required",
                        message: "first frame must be register".to_string(),
                    },
                )
                .await;
                return;
            }
            Err(error) => {
                send_frame(
                    &mut socket,
                    ServerFrame::Error {
                        request_id: None,
                        code: "invalid_json",
                        message: error.to_string(),
                    },
                )
                .await;
                return;
            }
        },
        _ => return,
    };

    let peer_id = registration.peer.peer_id;
    if !send_frame(
        &mut socket,
        ServerFrame::Registered {
            protocol_version: LIVE_PROTOCOL_VERSION,
            peer: registration.peer,
        },
    )
    .await
    {
        hub.unregister(peer_id).await;
        return;
    }

    let mut receiver = registration.receiver;
    let (control_tx, mut control_rx) = mpsc::channel(PEER_QUEUE_CAPACITY);
    let in_flight_sends = Arc::new(Semaphore::new(MAX_IN_FLIGHT_SENDS_PER_PEER));
    let mut ping = tokio::time::interval(Duration::from_secs(30));
    ping.tick().await;
    loop {
        tokio::select! {
            _ = ping.tick() => {
                if socket.send(Message::Ping(Vec::new())).await.is_err() {
                    break;
                }
            }
            outbound = receiver.recv() => {
                let Some(message) = outbound else { break };
                if !send_frame(&mut socket, ServerFrame::Message { message }).await {
                    break;
                }
            }
            control = control_rx.recv() => {
                let Some(frame) = control else { break };
                if !send_frame(&mut socket, frame).await {
                    break;
                }
            }
            inbound = socket.recv() => {
                let Some(Ok(inbound)) = inbound else { break };
                match inbound {
                    Message::Text(text) => {
                        let frame = match serde_json::from_str::<ClientFrame>(&text) {
                            Ok(frame) => frame,
                            Err(error) => {
                                send_frame(&mut socket, ServerFrame::Error {
                                    request_id: None,
                                    code: "invalid_json",
                                    message: error.to_string(),
                                }).await;
                                continue;
                            }
                        };
                        match frame {
                            ClientFrame::Register { .. } => {
                                send_frame(&mut socket, ServerFrame::Error {
                                    request_id: None,
                                    code: "already_registered",
                                    message: "this connection is already registered".to_string(),
                                }).await;
                            }
                            ClientFrame::Acknowledge { message_id, accepted } => {
                                if let Err(error) = hub.acknowledge(peer_id, message_id, accepted).await {
                                    send_frame(&mut socket, ServerFrame::error(None, error)).await;
                                }
                            }
                            ClientFrame::ListPeers { request_id } => {
                                let peers = hub.list().await;
                                if !send_frame(&mut socket, ServerFrame::Peers { request_id, peers }).await {
                                    break;
                                }
                            }
                            ClientFrame::SendMessage { request_id, to_peer_id, body, in_reply_to } => {
                                let permit = match in_flight_sends.clone().try_acquire_owned() {
                                    Ok(permit) => permit,
                                    Err(_) => {
                                        let error = LiveError::Busy(format!(
                                            "too many live sends in flight (max {MAX_IN_FLIGHT_SENDS_PER_PEER})"
                                        ));
                                        send_frame(
                                            &mut socket,
                                            ServerFrame::error(Some(request_id), error),
                                        ).await;
                                        continue;
                                    }
                                };
                                let send_hub = hub.clone();
                                let send_agent = agent_name.clone();
                                let response_tx = control_tx.clone();
                                tokio::spawn(async move {
                                    let _permit = permit;
                                    let response = match send_hub.send_idempotent(SendSpec {
                                        bound_agent: &send_agent,
                                        from_peer_id: peer_id,
                                        to_peer_id,
                                        body: &body,
                                        in_reply_to,
                                        idempotency_key: request_id,
                                        source_binding: "connection_bound",
                                    }).await {
                                        Ok(receipt) => ServerFrame::SendResult { request_id, receipt },
                                        Err(error) => ServerFrame::error(Some(request_id), error),
                                    };
                                    let _ = response_tx.send(response).await;
                                });
                            }
                        }
                    }
                    Message::Close(_) => break,
                    Message::Ping(_) | Message::Pong(_) | Message::Binary(_) => {}
                }
            }
        }
    }
    hub.unregister(peer_id).await;
}

async fn send_frame(socket: &mut WebSocket, frame: ServerFrame) -> bool {
    match serde_json::to_string(&frame) {
        Ok(json) => socket.send(Message::Text(json)).await.is_ok(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn pair(hub: &LiveHub) -> (LiveRegistration, LiveRegistration) {
        let claude = hub
            .register("CC-Stealth", LiveAdapter::ClaudeCode, "claude-1")
            .await
            .unwrap();
        let codex = hub
            .register("Codex-Stealth", LiveAdapter::Codex, "codex-1")
            .await
            .unwrap();
        (claude, codex)
    }

    #[tokio::test]
    async fn lists_only_connected_ephemeral_peers() {
        let hub = LiveHub::default();
        let (claude, codex) = pair(&hub).await;
        assert_eq!(hub.list().await.len(), 2);
        hub.unregister(claude.peer.peer_id).await;
        assert_eq!(hub.list().await, vec![codex.peer]);
    }

    #[tokio::test]
    async fn delivery_reports_host_acceptance() {
        let hub = LiveHub::default();
        let (claude, mut codex) = pair(&hub).await;
        let receiver_hub = hub.clone();
        let codex_id = codex.peer.peer_id;
        tokio::spawn(async move {
            let message = codex.receiver.recv().await.unwrap();
            receiver_hub
                .acknowledge(codex_id, message.message_id, true)
                .await
                .unwrap();
        });
        let receipt = hub
            .send(
                "CC-Stealth",
                claude.peer.peer_id,
                codex_id,
                "hello from Claude",
                None,
            )
            .await
            .unwrap();
        assert_eq!(receipt.status, DeliveryStatus::HostAccepted);
    }

    #[tokio::test]
    async fn sender_identity_is_server_bound() {
        let hub = LiveHub::default();
        let (claude, codex) = pair(&hub).await;
        let error = hub
            .send(
                "Forged-Agent",
                claude.peer.peer_id,
                codex.peer.peer_id,
                "forged",
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, LiveError::Unauthorized(_)));
    }

    #[tokio::test]
    async fn agent_level_send_requires_exactly_one_source_peer() {
        let hub = LiveHub::default();
        let (claude, mut codex) = pair(&hub).await;
        let ack_hub = hub.clone();
        let codex_id = codex.peer.peer_id;
        tokio::spawn(async move {
            let message = codex.receiver.recv().await.unwrap();
            ack_hub
                .acknowledge(codex_id, message.message_id, true)
                .await
                .unwrap();
        });
        assert_eq!(
            hub.send_from_agent(
                "CC-Stealth",
                codex_id,
                "unique source",
                None,
                Uuid::now_v7(),
            )
            .await
            .unwrap()
            .status,
            DeliveryStatus::HostAccepted
        );

        hub.register("CC-Stealth", LiveAdapter::ClaudeCode, "claude-2")
            .await
            .unwrap();
        let ambiguous = hub
            .send_from_agent(
                "CC-Stealth",
                codex_id,
                "must not guess",
                None,
                Uuid::now_v7(),
            )
            .await
            .unwrap_err();
        assert!(matches!(ambiguous, LiveError::Invalid(_)));
        drop(claude);
    }

    #[tokio::test]
    async fn offline_messages_are_not_queued() {
        let hub = LiveHub::default();
        let (claude, codex) = pair(&hub).await;
        hub.unregister(codex.peer.peer_id).await;
        let error = hub
            .send(
                "CC-Stealth",
                claude.peer.peer_id,
                codex.peer.peer_id,
                "use a handoff",
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(error, LiveError::NotFound(_)));
        assert!(error.to_string().contains("handoff"));
    }

    #[tokio::test]
    async fn repeated_idempotency_key_is_suppressed() {
        let hub = LiveHub::default();
        let (claude, mut codex) = pair(&hub).await;
        let receiver_hub = hub.clone();
        let codex_id = codex.peer.peer_id;
        tokio::spawn(async move {
            if let Some(message) = codex.receiver.recv().await {
                receiver_hub
                    .acknowledge(codex_id, message.message_id, true)
                    .await
                    .unwrap();
            }
        });
        let idempotency_key = Uuid::now_v7();
        hub.send_idempotent(SendSpec {
            bound_agent: "CC-Stealth",
            from_peer_id: claude.peer.peer_id,
            to_peer_id: codex_id,
            body: "same message",
            in_reply_to: None,
            idempotency_key,
            source_binding: "connection_bound",
        })
        .await
        .unwrap();
        let error = hub
            .send_idempotent(SendSpec {
                bound_agent: "CC-Stealth",
                from_peer_id: claude.peer.peer_id,
                to_peer_id: codex_id,
                body: "same message",
                in_reply_to: None,
                idempotency_key,
                source_binding: "connection_bound",
            })
            .await
            .unwrap_err();
        assert!(matches!(error, LiveError::Duplicate(_)));
    }

    #[tokio::test]
    async fn idempotency_survives_source_reconnect() {
        let hub = LiveHub::default();
        let (claude, mut codex) = pair(&hub).await;
        let codex_id = codex.peer.peer_id;
        let key = Uuid::now_v7();
        let ack_hub = hub.clone();
        tokio::spawn(async move {
            let message = codex.receiver.recv().await.unwrap();
            ack_hub
                .acknowledge(codex_id, message.message_id, true)
                .await
                .unwrap();
        });
        hub.send_from_agent("CC-Stealth", codex_id, "once", None, key)
            .await
            .unwrap();
        hub.unregister(claude.peer.peer_id).await;
        hub.register("CC-Stealth", LiveAdapter::ClaudeCode, "claude-new")
            .await
            .unwrap();
        let duplicate = hub
            .send_from_agent("CC-Stealth", codex_id, "once", None, key)
            .await
            .unwrap_err();
        assert!(matches!(duplicate, LiveError::Duplicate(_)));
    }

    #[tokio::test]
    async fn acknowledgement_must_come_from_exact_target_peer() {
        let hub = LiveHub::default();
        let (claude, mut codex) = pair(&hub).await;
        let sender_hub = hub.clone();
        let from_id = claude.peer.peer_id;
        let target_id = codex.peer.peer_id;
        let send = tokio::spawn(async move {
            sender_hub
                .send("CC-Stealth", from_id, target_id, "verify ack source", None)
                .await
        });
        let message = codex.receiver.recv().await.unwrap();
        let spoof = hub
            .acknowledge(from_id, message.message_id, true)
            .await
            .unwrap_err();
        assert!(matches!(spoof, LiveError::NotFound(_)));
        hub.acknowledge(target_id, message.message_id, true)
            .await
            .unwrap();
        assert_eq!(
            send.await.unwrap().unwrap().status,
            DeliveryStatus::HostAccepted
        );
    }

    #[tokio::test]
    async fn target_rejection_is_standardized() {
        let hub = LiveHub::default();
        let (claude, mut codex) = pair(&hub).await;
        let sender_hub = hub.clone();
        let from_id = claude.peer.peer_id;
        let target_id = codex.peer.peer_id;
        let send = tokio::spawn(async move {
            sender_hub
                .send("CC-Stealth", from_id, target_id, "reject me", None)
                .await
        });
        let message = codex.receiver.recv().await.unwrap();
        hub.acknowledge(target_id, message.message_id, false)
            .await
            .unwrap();
        let error = send.await.unwrap().unwrap_err();
        assert_eq!(
            error,
            LiveError::Rejected("target adapter rejected the message".to_string())
        );
    }

    #[tokio::test]
    async fn rejects_self_send_and_oversized_text() {
        let hub = LiveHub::default();
        let (claude, codex) = pair(&hub).await;
        let self_send = hub
            .send(
                "CC-Stealth",
                claude.peer.peer_id,
                claude.peer.peer_id,
                "loop",
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(self_send, LiveError::Invalid(_)));

        let oversized = "x".repeat(MAX_MESSAGE_BYTES + 1);
        let too_large = hub
            .send(
                "CC-Stealth",
                claude.peer.peer_id,
                codex.peer.peer_id,
                &oversized,
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(too_large, LiveError::Invalid(_)));
    }

    #[tokio::test]
    async fn reconnect_gets_a_new_peer_id_and_no_resume_state() {
        let hub = LiveHub::default();
        let first = hub
            .register("Codex-Stealth", LiveAdapter::Codex, "codex-1")
            .await
            .unwrap();
        let first_id = first.peer.peer_id;
        hub.unregister(first_id).await;
        let second = hub
            .register("Codex-Stealth", LiveAdapter::Codex, "codex-1")
            .await
            .unwrap();
        assert_ne!(first_id, second.peer.peer_id);
        assert_eq!(hub.list().await, vec![second.peer]);
        assert!(LiveHub::default().list().await.is_empty());
    }

    #[test]
    fn host_allowlist_is_exact_and_case_insensitive() {
        let hosts = vec!["localhost:3000".to_string(), "ops.example.com".to_string()];
        assert!(host_allowed("LOCALHOST:3000", &hosts));
        assert!(host_allowed("ops.example.com", &hosts));
        assert!(host_allowed("ops.example.com:443", &hosts));
        assert!(!host_allowed("localhost:3001", &hosts));
        assert!(!host_allowed("ops.example.com.evil", &hosts));
    }

    #[tokio::test]
    async fn registration_rejects_unsafe_labels_and_caps_each_agent() {
        let hub = LiveHub::default();
        assert!(matches!(
            hub.register("Codex-Stealth", LiveAdapter::Codex, "bad\nlabel")
                .await,
            Err(LiveError::Invalid(_))
        ));
        for index in 0..MAX_PEERS_PER_AGENT {
            hub.register(
                "Codex-Stealth",
                LiveAdapter::Codex,
                &format!("codex-{index}"),
            )
            .await
            .unwrap();
        }
        assert!(matches!(
            hub.register("Codex-Stealth", LiveAdapter::Codex, "one-too-many")
                .await,
            Err(LiveError::Busy(_))
        ));
    }

    #[test]
    fn only_agent_tokens_can_register_live() {
        use crate::auth::{AgentToken, MachineToken};

        assert_eq!(
            live_agent_name(CallerClass::Full),
            Err(StatusCode::FORBIDDEN)
        );
        assert_eq!(
            live_agent_name(CallerClass::Agent(Arc::new(AgentToken {
                token: "test".to_string(),
                from_agent: "Codex-Stealth".to_string(),
                client: None,
            }))),
            Ok("Codex-Stealth".to_string())
        );
        assert_eq!(
            live_agent_name(CallerClass::Machine(Arc::new(MachineToken {
                token: "test".to_string(),
                from_agent: "Producer".to_string(),
                client: None,
                agents: vec!["Codex-Stealth".to_string()],
                scopes: vec!["read".to_string()],
            }))),
            Err(StatusCode::FORBIDDEN)
        );
    }

    #[test]
    fn protocol_accepts_only_claude_code_and_codex_adapters() {
        let claude = r#"{"type":"register","protocol":1,"adapter":"claude_code","label":"one"}"#;
        let codex = r#"{"type":"register","protocol":1,"adapter":"codex","label":"two"}"#;
        let antigravity =
            r#"{"type":"register","protocol":1,"adapter":"antigravity","label":"three"}"#;
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(claude),
            Ok(ClientFrame::Register {
                adapter: LiveAdapter::ClaudeCode,
                ..
            })
        ));
        assert!(matches!(
            serde_json::from_str::<ClientFrame>(codex),
            Ok(ClientFrame::Register {
                adapter: LiveAdapter::Codex,
                ..
            })
        ));
        assert!(serde_json::from_str::<ClientFrame>(antigravity).is_err());
    }
}
