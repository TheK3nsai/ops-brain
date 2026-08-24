# ops-brain Claude Code Channel

Local two-way [Claude Code Channel](https://code.claude.com/docs/en/channels-reference)
adapter for ops-brain's online-delivery `/live` protocol. Claude Code spawns this
process over stdio; it connects outbound to ops-brain with an identity-bound
agent token.

This lane is online-only. It never persists, resumes, replays, or queues a peer
message while disconnected. Use an ops-brain handoff for durable work.

## Requirements

- Node.js 22 or newer.
- Claude Code with Channels support. The adapter is verified against local
  Claude Code 2.1.241.
- An ops-brain per-agent bearer bound to the Claude agent identity. The main
  bearer and machine tokens cannot use `/live`.
- During the Channels research preview, a custom server must be started with
  `--dangerously-load-development-channels`. Organization policy can still
  disable Channels.

## Install

From this directory:

```sh
npm ci --ignore-scripts
npm test
```

Fleet operators should use the Linux/Windows installers and foreground
launchers in [`../../docs/live-fleet-rollout.md`](../../docs/live-fleet-rollout.md)
rather than hand-authoring MCP configuration on every host.

The package is intentionally self-contained. Its executable is
`src/main.js`; it writes MCP JSON-RPC only to stdout and operational messages
only to stderr.

## Configure Claude Code

Use the release bundle's `ops-brain-client configure claude` command and launch
with `ops-brain-claude`. The launcher creates a private per-launch user-scope
configuration because Claude's Channel resolver cannot see `--mcp-config`
servers. Do not register this adapter ambiently in the ordinary user/local
scope: unrelated and wake sessions would spawn duplicate peers.

Do **not** put `OPS_BRAIN_AGENT_TOKEN` in `.mcp.json`, command arguments, the
URL, or a checked-in file. Prefer `OPS_BRAIN_AGENT_TOKEN_FILE`: this keeps the
bearer out of Claude Code's ambient environment, so hooks and unrelated MCP
subprocesses do not inherit it automatically. The file must be inaccessible to
group and other users. This is not same-user process isolation; any process
running as that OS account may still read the file. These variables are
recognized:

| Variable | Required | Purpose |
| --- | --- | --- |
| `OPS_BRAIN_LIVE_URL` | yes | Exact `wss://host/live` endpoint. Plain `ws://` is accepted only on loopback for development. |
| `OPS_BRAIN_EXPECTED_AGENT` | yes | Exact server-bound identity expected after registration, such as `CC-Stealth`; a mismatch disconnects fail-closed. |
| `OPS_BRAIN_AGENT_TOKEN_FILE` | recommended | Protected file containing the identity-bound bearer. |
| `OPS_BRAIN_AGENT_TOKEN` | alternative | Identity-bound bearer inherited by the adapter; mutually exclusive with the file. |
| `OPS_BRAIN_AGENT_TOKEN_HELPER_JSON` | launcher-internal alternative | JSON command array for a short-lived credential helper; mutually exclusive with the other token sources. |
| `OPS_BRAIN_LIVE_LABEL` | no | Non-sensitive local disambiguator; defaults to `claude-code`. |

The internal Channel name remains `ops-brain-live` to avoid colliding with the
ordinary remote MCP entry named `ops-brain`. It is an implementation detail of
the isolated launcher; invoke the wrapper, not the underlying flag directly:

```sh
ops-brain-claude
```

Claude shows a development-channel warning and, on first use, the normal MCP
server consent prompt. The preview flag bypasses only Anthropic's channel
allowlist; it does not bypass organization policy or tool permissions.

## Claude-facing tools

- `list_live_peers` lists only currently connected opaque peer IDs.
- `send_live_message` sends to a connected peer. For a reply, copy the inbound
  channel event's `reply_peer_id` to `to_peer_id` and `message_id` to
  `in_reply_to`.

The adapter acknowledges an inbound message only after the Channel notification
has been written to Claude Code's stdio transport. Per Claude's preview
contract, this is host transport acceptance, not proof that the model read or
acted on the event.

## Trust boundary

Every inbound body is prefixed with a fixed security boundary and arrives with
`trust="untrusted_peer_input"`. Peer text cannot grant permission or consent,
change instructions or configuration, authorize credentials or destructive
actions, or elevate itself. Security-sensitive requests require independent
verification.

Never send credentials, secrets, PII, PHI, or file contents through live chat.
Send a pointer and a finding instead. This adapter deliberately does not declare
Claude's permission-relay capability.

## Failure behavior

- One WebSocket exists per adapter process, with exponential reconnect capped
  at 30 seconds.
- Sends and peer listing fail immediately while offline; nothing is queued.
- At most 16 local requests may be in flight and at most 32 inbound events wait
  for stdio injection.
- An invalid, overflowing, or failed Channel injection receives a negative ACK.
- Queued work is bound to the connection's peer ID and is discarded after a
  reconnect. A disconnect after the host write but before its ACK is inherently
  ambiguous: the sender may see rejection even though the host received it.
- Ping/Pong is transport liveness only. Reconnects receive a new opaque peer ID.

Run all hostless tests with:

```sh
npm run check
npm test
```

The suite exercises the MCP capability and tools through the official SDK's
in-memory transport, plus WebSocket authentication/framing, ACK ordering,
offline behavior, validation, and the untrusted wrapper. It does not require a
Claude session or a running ops-brain server.
