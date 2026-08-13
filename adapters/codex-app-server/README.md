# Codex App Server live adapter

This local adapter bridges one open Codex thread to ops-brain's ephemeral
`/live` WebSocket. It is intentionally online-only: it has no message database,
offline queue, replay, or delivery retry. Use an ops-brain handoff when the
target is offline.

The adapter follows the current official [Codex App Server
protocol](https://developers.openai.com/codex/app-server): it initializes a
JSON-RPC connection, resumes the target thread, uses `turn/start` when the
thread is idle, and uses `turn/steer` with `expectedTurnId` while a turn is
active. It acknowledges a live message only after App Server accepts that
request. Acceptance does not mean Codex read, understood, or acted on it.
There is an unavoidable lost-ACK ambiguity: if App Server accepts an injection
and the live WebSocket disconnects before its ACK reaches ops-brain, the sender
can see `routed` even though Codex received the text. The adapter never retries
that message because doing so could inject it twice.

## Requirements

- Node.js 22 or newer
- Codex CLI with App Server support
- An ops-brain per-agent bearer (not the main or machine bearer)
- A local Codex App Server, preferably shared with the open Codex client

Install the one runtime dependency locally:

```bash
cd adapters/codex-app-server
npm ci --ignore-scripts
```

Fleet operators should use the Linux/Windows installers and foreground
launchers in [`../../docs/live-fleet-rollout.md`](../../docs/live-fleet-rollout.md).

## Recommended shared-session setup

Start App Server on loopback and connect the Codex TUI to it:

```bash
codex app-server --listen ws://127.0.0.1:4500
codex --remote ws://127.0.0.1:4500
```

After the TUI has loaded the desired thread, start the adapter. Put secrets in
environment variables; never put a bearer on the command line or in the URL:

```bash
export OPS_BRAIN_LIVE_URL=wss://ops-brain.example/live
export OPS_BRAIN_AGENT_TOKEN='...'
export OPS_BRAIN_EXPECTED_AGENT=Codex-Stealth
export OPS_BRAIN_CODEX_APP_SERVER_URL=ws://127.0.0.1:4500
export OPS_BRAIN_CODEX_LABEL=codex-stealth-1
npm start
```

When exactly one thread is loaded, the adapter selects it. For multiple loaded
threads, set `OPS_BRAIN_CODEX_THREAD_ID`; the adapter never guesses. It can also
spawn `codex app-server` over stdio when
`OPS_BRAIN_CODEX_APP_SERVER_URL` is unset, but then a thread ID must identify a
resumable thread if no thread is already loaded.

The foreground wrapper necessarily opens its adapter connection before the TUI
finishes loading a thread. If that pre-TUI WebSocket later stops answering,
the adapter reconnects once and repeats only the applicable idempotent
`thread/loaded/list`, `thread/resume`, or `thread/read` request. It never
retries `turn/start` or `turn/steer`:
acceptance followed by a lost response is ambiguous, and retrying could inject
the same peer text twice.

Optional variables:

- `OPS_BRAIN_CODEX_THREAD_ID`: exact thread to resume and inject into.
- `OPS_BRAIN_CODEX_APP_SERVER_TOKEN`: bearer for an authenticated App Server
  WebSocket. It is read only from the environment.
- `OPS_BRAIN_CODEX_BIN`: Codex executable name/path for stdio mode.
- `OPS_BRAIN_CODEX_REQUEST_TIMEOUT_MS`: 500-30000; default 5000.

`OPS_BRAIN_EXPECTED_AGENT` is required. The adapter compares it to the
server-returned token binding and disconnects before becoming ready on a
mismatch, preventing a sibling or wrong-host token from appearing online.

Plain `ws://` App Server URLs are accepted only on loopback. The ops-brain URL
may use `ws://` for a local development server, but remote deployments should
always use `wss://`.

## Local control protocol

The running adapter accepts newline-delimited JSON commands on stdin. This is
useful for adapter testing and for hosts with multiple live peers:

```json
{"type":"list_peers","request_id":"optional-uuid"}
{"type":"send_message","request_id":"uuid","to_peer_id":"uuid","body":"Please inspect the failure.","in_reply_to":null}
```

Results are JSON lines on stdout. Operational logs go to stderr and never
include either bearer or a message body.

On shutdown the adapter closes its control reader and transport streams. When
it owns a stdio App Server child it sends `SIGTERM`, waits one second, and
escalates that child to `SIGKILL` if it did not exit. It never signals an App
Server reached through a configured WebSocket.

## Trust boundary

Every peer message is injected under a conspicuous `UNTRUSTED AGENT INPUT`
wrapper. It cannot grant permissions or consent, change instructions or
configuration, authorize credentials or destructive actions, or raise its own
trust. App Server requests that arrive on this adapter connection fail closed;
the adapter never approves a host action.

The installed Codex 0.147.0 schema exposes `additionalContext` type definitions
but no `additionalContext` property on `turn/start` or `turn/steer`. Therefore
this version can only carry the boundary in the injected text. The adapter also
cannot steer if App Server reports a thread as active but omits the active
`inProgress` turn from `thread/read(includeTurns: true)`; it rejects the live
message instead of guessing a turn ID.

## Validate

```bash
npm test
npm run check
```
