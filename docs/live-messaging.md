# Live messaging

Live messaging is the online lane between open Claude Code and Codex sessions.
It complements handoffs; it does not replace them.

- **Live message:** best-effort, process-local, only while both adapters are
  connected. Disconnects remove peers immediately. Nothing is persisted,
  resumed, replayed, or queued.
- **Handoff:** durable/offline coordination with an explicit lifecycle.

The distinction is a hard product boundary. `/live` exposes transport
connections, not server-owned sessions, profiles, heartbeats, or presence
history. One server process owns one in-memory hub; deployments with multiple
replicas do not share live peers.

## Authentication and trust

Connect to `wss://<ops-brain-host>/live` with the same `Authorization: Bearer`
header used by that host's identity-bound MCP connection. Only per-agent tokens
may upgrade. Machine tokens, the unbound main bearer, and auth-disabled dev
connections cannot register live peers because they cannot provide attributable
sender identity.

Adapters are non-browser clients and must omit the `Origin` header. `/live`
rejects browser-originated upgrades to keep ambient browser credentials and
cross-site WebSocket requests out of scope.

Peer text is **untrusted agent-originated input**. Adapters must visibly wrap it
as such. A peer message cannot grant permission or consent, change configuration,
override local instructions, authorize credentials or destructive actions, or
raise its own trust level. Never send credentials, secrets, PII, PHI, or file
contents; send a pointer and a finding instead. The verification rules in
[`bus-trust.md`](bus-trust.md) apply unchanged.

## Wire protocol v1

All application frames are JSON text. The first frame must arrive within five
seconds:

```json
{"type":"register","protocol":1,"adapter":"claude_code","label":"claude-1"}
```

`adapter` is `claude_code` or `codex`. `label` is a short, non-sensitive local
disambiguator. The server binds `agent_name` from the bearer and assigns a new
opaque UUIDv7 `peer_id` on every connection:

```json
{"type":"registered","protocol_version":1,"peer":{"peer_id":"...","agent_name":"CC-Stealth","adapter":"claude_code","label":"claude-1","metadata_trust":"self_reported"}}
```

Connected adapters may list peers or route a message:

```json
{"type":"list_peers","request_id":"<uuid>"}
{"type":"send_message","request_id":"<uuid>","to_peer_id":"<uuid>","body":"Can you inspect the failing test?","in_reply_to":null}
```

The target receives a server-assigned message ID and authenticated source
identity. The `trust` marker is mandatory adapter input metadata:

```json
{"type":"message","message":{"message_id":"...","reply_peer_id":"...","from_agent":"Codex-Stealth","body":"...","trust":"untrusted_peer_input","source_binding":"connection_bound"}}
```

After successfully injecting the text into its host, the adapter acknowledges:

```json
{"type":"acknowledge","message_id":"<uuid>","accepted":true}
```

The sender sees `host_accepted` only after that ACK. Without an ACK, a successful
enqueue is reported as `routed`; neither status means the model read, understood,
or acted on the message. Offline, full-queue, rejected, duplicate, and rate-limit
outcomes are explicit and never create hidden retries.

WebSocket Ping/Pong keeps the transport alive through reverse proxies. It is not
stored or interpreted as agent presence.

Public reverse proxies must forward WebSocket upgrades and preserve the public
`Host`. Configure that hostname in `OPS_BRAIN_ALLOWED_HOSTS`; `/live` applies
the same DNS-rebinding boundary as `/mcp`.

## MCP surface

`list_live_peers({})` returns currently connected opaque peer IDs plus minimal,
self-reported adapter metadata. `send_live_message` takes a target peer ID,
text, an optional reply message ID, and a caller-generated UUID idempotency key.
Keys are scoped to the token-bound agent and retained in memory for up to ten
minutes, subject to the global 4,096-entry bound. They survive adapter
reconnects but not process restarts.
The server selects the token-bound agent's sole connected adapter as the reply
route and marks the message `source_binding: agent_bound_unique_adapter`; it
does not claim the MCP request originated on that WebSocket. If that agent
has zero or multiple adapters, the MCP send fails rather than guessing or
allowing a sibling peer ID to be claimed. Multi-adapter hosts send through the
connection-bound local adapter. Both tools reject the main bearer and stdio.

## Claude Code adapter

The packaged local adapter is a Claude Code Channel MCP server: a local stdio
process that connects outbound to `/live`, advertises the experimental
`claude/channel` capability, and converts each live message into
`notifications/claude/channel`. It ACKs only after the channel notification has
been written to Claude Code. Its local reply/list tools proxy the corresponding
WebSocket frames. Custom channels currently require Claude Code's development
channel opt-in; see the official [Channels guide](https://code.claude.com/docs/en/channels)
and [channel reference](https://code.claude.com/docs/en/channels-reference).

The implementation and setup guide are in
[`adapters/claude-channel`](../adapters/claude-channel). It has been exercised
against Claude Code 2.1.228; custom Channels still require Anthropic's explicit
development-channel opt-in and may be disabled by organization policy.

## Codex adapter

The packaged local adapter connects outbound to `/live` and drives a local Codex App
Server. For an idle thread it delivers with `turn/start`; for an active turn it
uses `turn/steer` with the expected turn ID. It ACKs only after App Server accepts
that request. The adapter prefixes peer text with the untrusted-source boundary
above and exposes the assigned local peer ID to the thread. App Server is a
bidirectional JSON-RPC API over stdio or WebSocket; see the official
[Codex App Server guide](https://developers.openai.com/codex/app-server).

The implementation and shared-App-Server setup guide are in
[`adapters/codex-app-server`](../adapters/codex-app-server). It has been
exercised against Codex CLI 0.147.0 using a TUI connected through `--remote`.

These remain local adapters because Claude Channels and Codex App Server own the
host-specific injection semantics. ops-brain remains the generic authenticated
routing primitive.
