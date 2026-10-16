# Optional handoff events

Set `OPS_BRAIN_EVENTS_ENABLED=true` to enable an additive MCP2 HTTP adapter
and background webhook delivery on the existing authenticated `/mcp` endpoint.
It supports protocol `2026-07-28` discovery, the existing 13 tools, and the
`handoff.created` webhook event. Legacy initialization and tools continue
through rmcp. The adapter advertises only its implemented capabilities.

This implements passive handoff notifications. The receiving application
decides whether and when to run work. Existing machine polling remains
available. No agent profiles, durable agent sessions or orchestration are
introduced. [OpenAI MCP Events documentation](https://developers.openai.com/plugins/build/mcp-events).

## What can be subscribed to

Authenticated agent seats may subscribe only to new handoffs explicitly
addressed to their bound `agent_name`, or a concrete matching member when the
credential binds a slug family. Each subscription watches that one member.
A subscription cannot watch other
queues or broadcasts. Restricted seats retain their existing retrieval
boundary. Operator and machine credentials cannot create subscriptions.

Delivered application data contains only:

```json
{"handoff_id":"<handoff UUID>"}
```

The receiver fetches the full record using the ordinary authenticated
`get_handoff` tool. Handoff bodies and callback credentials never appear in
the notification payload. Agent and machine INSERT paths enqueue events in
the same database transaction as the handoff; replaying an MCP retry key does
not insert another handoff or event.

## Subscription lifecycle

MCP2 clients use `server/discover`, `events/list`, `events/subscribe` and
`events/unsubscribe`. They supply a webhook callback and a `whsec_` signing
secret whose base64 value decodes to 24–64 bytes. Protocol metadata and HTTP
method/name/version headers must agree.

Before activating a callback, the server sends a signed, random verification
challenge and requires the matching echo. URLs must use HTTPS and resolve
only to public addresses. The server re-resolves and validates destinations
for every send, pins the connection to those addresses, retains TLS hostname
verification, disables proxies and follows no redirects.

Equivalent subscriptions refresh the same deterministic ID. A principal has
at most sixteen active subscriptions. Lifetimes are capped at one day and,
for OAuth, by the access token's expiry. Refresh before `refreshBefore`;
indefinite grants are not issued. Refresh with a replacement secret signs
with both keys during a five-minute rotation window. Unsubscribe is
idempotent and prevents subsequent delivery after any in-flight send finishes.

Subscriptions and the outbox survive restarts. Each delivery rechecks the
credential binding, current visibility, expiration and subscription
generation. Static credential replacement invalidates its old subscriptions;
the new credential must subscribe again. OAuth shares the JWT revocation
limits documented in [oauth.md](oauth.md).

## Delivery and operations

Events use Standard Webhooks HMAC signatures, stable event IDs, occurrence
timestamps and subscription IDs. Delivery is at least once: a crash after a
receiver accepts can repeat an event. Receivers must deduplicate event IDs.
Network failures, 408, 429 and 5xx responses receive bounded exponential
retries, up to eight attempts. Other responses are terminal; 410 also
deactivates the subscription. No historical replay cursor is offered.

The database contains callback URLs and signing secrets. Restrict its access
and encrypt backups. Maintenance runs every five minutes: it clears expired
rotation keys, removes completed outbox rows after one day, and removes expired
subscriptions after seven days. Treat the event catalog as data,
and keep notifications free of behavioral instructions.

Disabling the flag stops the adapter and delivery worker. Database state
remains until expiry/cleanup; re-enabling before expiry can deliver queued
events. For a restored backup, keep events disabled and remove stale
subscriptions before reconnecting consumers.

Local tests cover protocol dispatch, identity/visibility boundaries,
subscription authorization, outbox generation and delivery primitives. Live
ChatGPT/Dots subscriptions have not been tested. An enabled, connected plugin
and supported consumer are required; the server flag alone cannot start a Dot.
