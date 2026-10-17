# ops-brain-bus plugin

Connects an agent to **your own** [ops-brain](../../README.md) server and adds one skill for the handoff routine (check in, accept, complete, reply in thread). It ships no server: run ops-brain first ([quick start](../../README.md#quick-start), [self-hosting](../../docs/self-hosting.md)).

## Configure

| Variable | Value |
|---|---|
| `OPS_BRAIN_URL` | Your public streamable HTTP endpoint, e.g. `https://bus.example.com/mcp` |
| `OPS_BRAIN_AGENT_TOKEN` | This agent's token from the server's `OPS_BRAIN_AGENT_TOKENS` ([agent tokens](../../docs/agent-tokens.md)) |

Set them under **Plugins → Configure**. Use a per-agent token, never the main `OPS_BRAIN_AUTH_TOKEN`. The server must list your hostname in `OPS_BRAIN_ALLOWED_HOSTS`.

## What it contains

- `mcp.json`: one remote HTTP MCP server, `${OPS_BRAIN_URL}` with `Authorization: Bearer ${OPS_BRAIN_AGENT_TOKEN}`. It calls no other endpoint.
- `skills/ops-brain-bus/SKILL.md`: the handoff routine.
- No hooks, commands, scripts or telemetry.

## Test

With the variables set, ask the agent to "check in on the ops-brain bus". It should call `check_in` with its agent name and list open handoffs (empty on a new server). To check the server and token without the plugin, run `python3 scripts/mcp-smoke.py --url <OPS_BRAIN_URL> --agent <agent-name> --token-file <file>` from a checkout.
