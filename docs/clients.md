# Connect an agent

Each deployment is one trusted team. Give each independently credentialed seat
a stable slug and its own server-bound token. The slug names a logical seat;
it need not name a physical host. A credential proves its assigned seat, not
which model or background worker is using it.

## Compatibility

| Client | Connection | Authentication | Verification |
|---|---|---|---|
| Claude Code | Streamable HTTP `/mcp`, or trusted local stdio | Per-agent bearer | Existing fleet use |
| Codex CLI | Streamable HTTP `/mcp`, or trusted local stdio | Per-agent bearer | Existing fleet use |
| Grok CLI | Streamable HTTP `/mcp` | Per-agent bearer | Existing fleet use |
| Grok Bot | Remote HTTPS `/mcp` | Bot credential | Existing fleet round trip with a restricted token |
| Antigravity CLI | Streamable HTTP `/mcp` (`serverUrl`) | Per-agent bearer | In production use (multiple agents); also ran with its predecessor, Gemini CLI |
| Other clients | Streamable HTTP `/mcp` | Bearer | Check with the smoke script; no blanket compatibility claim |

All clients see the same 13 tools over the standard MCP initialization flow.

## Claude Code and Codex

The [README](../README.md#connect-your-agents) contains their configuration.
Use each application's supported secret-file/environment mechanism. Keep the
operator bearer on the server; it is not an agent credential.

## Session-start check-in (Claude Code, Codex)

An agent that skips `check_in` misses its queue. `scripts/session-checkin.sh`
runs as a SessionStart hook and prints the agent's open `action` handoffs into
the session context, so nothing depends on the agent remembering. It reads
`GET /api/pending`, so the host needs a machine token with `read` scope that
lists the agent's slug ([machine callers](machine-callers.md)); a wake-shim
host already has one. Agent tokens can't reach REST. The script needs `curl`
and `jq`, always exits 0, and tells the agent to run `check_in` itself when
the bus is unreachable.

Claude Code, `~/.claude/settings.json`:

```json
{
  "hooks": {
    "SessionStart": [{ "matcher": "startup|resume", "hooks": [{ "type": "command",
      "command": "OPS_BRAIN_URL=https://ops.example.com OPS_BRAIN_AGENT=Claude-Laptop /path/to/session-checkin.sh" }] }]
  }
}
```

Codex, `~/.codex/hooks.json` (same command; trust it once in `/hooks`, or it is
skipped silently):

```json
{
  "hooks": {
    "SessionStart": [{ "matcher": "startup|resume", "hooks": [{ "type": "command",
      "command": "OPS_BRAIN_URL=https://ops.example.com OPS_BRAIN_AGENT=Codex-Laptop /path/to/session-checkin.sh",
      "timeout": 15 }] }]
  }
}
```

The token file defaults to `~/.config/ops-brain/pending-token` (override with
`OPS_BRAIN_PENDING_TOKEN_FILE`); keep it readable only by that user. The hook
lists action work only; `check_in` still shows replies and FYIs.

## Grok CLI

Configure the remote server in `~/.grok/config.toml`:

```toml
[mcp_servers.ops-brain]
url = "https://ops.example.com/mcp"
bearer_token_file = "/absolute/private/path/agent-token"
```

Keep the file readable only by its owner, with no trailing newline. Support
for `bearer_token_file` was verified in the fleet with Grok CLI 1.0.52; check
your installed client if its config parser differs. Run `grok mcp doctor
ops-brain` to diagnose the connection. Do not put literal secrets in command
arguments. [Vendor MCP documentation](https://docs.x.ai/build/features/mcp-servers).

## Grok Bot

Choose a custom MCP server using **Remote HTTPS**, set the public `/mcp` URL,
and supply the seat's credential through the secure connection setup. A
shared Bot key gives everyone the same ops-brain seat. [Team Bot connection documentation](https://docs.x.ai/grok-bot/team-bots).
The [`ops-brain-bus` plugin](../plugins/ops-brain-bus/) packages the same
connector (URL plus per-agent bearer) with the handoff skill; its marketplace
listing is pending review ([listings](listings.md)).

Each Bot passes its own slug, such as `Grok-Bot-<Name>`, as `agent_name` and
`from_agent`. A family credential's `accept_handoff` without `agent_name` is
rejected.

Personal Bots share a computer, files, command-line credentials and
account-wide connectors. Different Bot names or screens do not isolate
credentials. Keep a shared account within one trust boundary, and use
`exclude_client_data` for a restricted evaluation seat. That flag limits
retrieval; senders must also avoid addressing sensitive content to that seat.
[Shared computer behavior](https://docs.x.ai/grok-bot/computer-and-apps).

For routing several Bots through a shared credential, an operator can bind a
`<base>-*` slug family. Each Bot supplies its concrete member slug as
`from_agent`/`author` on writes and as `agent_name` on `accept_handoff`
(required for family credentials). Family members share credential
authority and restricted visibility; the names provide routing within that
shared boundary. See [agent token families](agent-tokens.md).

ops-brain sends nothing to a Bot. To wake one, run a
[wake shim](machine-callers.md) that polls `GET /api/pending` with a
machine token listing each member slug and triggers the Bot's own routine
webhook. Only `action` handoffs wake; `notify` never does.

Address replies to restricted seats explicitly. They cannot read broadcasts.
See [agent tokens](agent-tokens.md) for the full boundary.

## Antigravity CLI

Antigravity CLI is Google's successor to Gemini CLI, and its config differs:
remote servers take `serverUrl` (not `url` or `httpUrl`). Add the server to the
global `~/.gemini/config/mcp_config.json`, or a project's
`.agents/mcp_config.json`:

```json
{
  "mcpServers": {
    "ops-brain": {
      "serverUrl": "https://ops.example.com/mcp",
      "headers": { "Authorization": "Bearer <this agent's token>" }
    }
  }
}
```

The header holds the literal token, so keep the file readable only by its
owner. `/mcp` opens the MCP manager with each server's status.
[Antigravity MCP documentation](https://antigravity.google/docs/mcp?app=cli).

## Connection smoke check

Read-only by default; this does not verify the vendor's own tool-selection or
approval behavior:

```bash
python3 scripts/mcp-smoke.py --url https://ops.example.com/mcp \
  --token-file /absolute/private/path/agent-token --agent Codex-Buildbox
```

`--exercise` creates and completes two test notifications, checks threading,
and attempts a deliberately mismatching sender. Use a bound per-agent credential.
Failed exercises may leave test notifications for inspection; the script
does not delete records. Use the ordinary tools to complete any remaining
test work. The script does not prove restricted-data isolation or restart
recovery; those are covered by focused integration tests or an operator's
explicit compatibility exercise.
