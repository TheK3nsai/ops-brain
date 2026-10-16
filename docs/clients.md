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
| Grok CLI | Streamable HTTP `/mcp` | Per-agent bearer | Existing fleet use; vendor supports HTTP and OAuth |
| Grok Bot | Remote HTTPS `/mcp` | Bot credential, or individual OAuth sign-in | Existing fleet round trip with a restricted token |
| ChatGPT Dots | Remote MCP through a connected plugin | OAuth | Implementation tested locally; account linking and Dot execution not verified |
| Other clients | Streamable HTTP `/mcp` | Bearer or supported OAuth | Check with the smoke script; no blanket compatibility claim |

The original 13 tools remain available. Legacy clients use the existing MCP
initialization flow. Optional events use the separate MCP2 discovery flow on
the same endpoint; see [events](events.md).

## Claude Code and Codex

The [README](../README.md#connect-your-agents) contains their configuration.
Use each application's supported secret-file/environment mechanism. Keep the
operator bearer on the server; it is not an agent credential.

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
and supply the seat's credential through the secure connection setup. For
team use, OAuth can identify the person connecting; a shared Bot key gives
everyone the same ops-brain seat. [Team Bot connection documentation](https://docs.x.ai/grok-bot/team-bots).

Personal Bots share a computer, files, command-line credentials and
account-wide connectors. Different Bot names or screens do not isolate
credentials. Keep a shared account within one trust boundary, and use
`exclude_client_data` for a restricted evaluation seat. That flag limits
retrieval; senders must also avoid addressing sensitive content to that seat.
[Shared computer behavior](https://docs.x.ai/grok-bot/computer-and-apps).

For routing several Bots through a shared credential, an operator can bind a
`<base>-*` slug family. Each Bot supplies its concrete member slug, including
`agent_name` for claims and event subscriptions. Family members share credential
authority and restricted visibility; the names provide routing within that
shared boundary. See [agent token families](agent-tokens.md).

Address replies to restricted seats explicitly. They cannot read broadcasts.
See [agent tokens](agent-tokens.md) for the full boundary.

## ChatGPT Dots

Configure [OAuth](oauth.md), connect the remote MCP server through a plugin,
complete account linking, and enable the plugin for the Dot. The documented
OpenAI authenticated integration expects OAuth rather than a custom static
API key. Existing ChatGPT/plugin permissions and approvals still apply.
[Authentication](https://developers.openai.com/plugins/build/auth),
[Dots](https://learn.chatgpt.com/docs/dots).

One OAuth subject/client binding maps to one ops-brain seat. If multiple Dots
or workers share that connection, they share that identity. Do not infer a
unique Dot identity from the name supplied in tool arguments.

## Connection smoke check

Read-only by default; this does not verify the vendor's own tool-selection or
approval behavior:

```bash
python3 scripts/mcp-smoke.py --url https://ops.example.com/mcp \
  --token-file /absolute/private/path/agent-token --agent Codex-Buildbox
```

`--exercise` creates and completes two test notifications, checks threading,
replays an idempotent write, and attempts a deliberately mismatching sender.
Use a bound per-agent credential and a build advertising `idempotency_key`.
Failed exercises may leave test notifications for inspection; the script
does not delete records. Use the ordinary tools to complete any remaining
test work. The script does not prove restricted-data isolation or restart
recovery; those are covered by focused integration tests or an operator's
explicit compatibility exercise.
