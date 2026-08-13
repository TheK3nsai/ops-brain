# ops-brain

The team bus. An [MCP](https://modelcontextprotocol.io/) server that gives Claude Code and Codex a shared coordination surface for state that must cross sessions, machines, or agent vendors — durable handoffs, bounded knowledge, briefings, and experimental online live messaging.

ops-brain is **not** local truth. Inventory belongs in your config management. Tickets and incidents belong in your ticketing system. Monitoring belongs in your monitoring stack. Reach for ops-brain only when you genuinely need the rest of the team.

## Who this is for

Solo operators and small teams running **multiple AI agents across multiple machines or vendors** — especially Claude Code and Codex sessions that cannot otherwise reach each other. ops-brain is the shared surface they coordinate over. If you run a single agent on a single box, you almost certainly don't need this.

## Quick start

No clone required — grab the standalone compose file, set a token, run it. The bundled PostgreSQL has pgvector preinstalled; embeddings start disabled (FTS still works) and can be enabled by pointing at an OpenAI-compatible endpoint like [Ollama](https://ollama.ai/) serving `nomic-embed-text`.

```bash
curl -O https://raw.githubusercontent.com/TheK3nsai/ops-brain/main/docker-compose.example.yml
echo "OPS_BRAIN_AUTH_TOKEN=$(openssl rand -hex 32)" > .env
docker compose -f docker-compose.example.yml up -d
curl -fsS http://localhost:3000/ready && echo "ready"
```

Images are multi-arch (`linux/amd64`, `linux/arm64`) on [`ghcr.io/thek3nsai/ops-brain`](https://github.com/TheK3nsai/ops-brain/pkgs/container/ops-brain). Pin a specific version with `:vX.Y.Z` instead of `:latest`.

Other deployment shapes:

- **Building from source** (contributing or pinning a local change) — clone the repo and use [`docker-compose.yml`](docker-compose.yml), which builds the Dockerfile and runs the same bundled PostgreSQL stack as the example.
- **Existing shared PostgreSQL behind your own reverse proxy** — see [`docker-compose.prod.yml`](docker-compose.prod.yml).

## Plug it into your agent

ops-brain speaks MCP over either stdio (default) or HTTP. Most multi-machine setups want HTTP so several agents on different hosts can hit the same server.

**Claude Code** — add to `~/.claude.json` under `mcpServers`:

```json
"ops-brain": {
  "type": "http",
  "url": "https://your-host.example.com/mcp",
  "headers": { "Authorization": "Bearer $OPS_BRAIN_AUTH_TOKEN" }
}
```

**Codex CLI** uses the same HTTP MCP transport through its own config — point it at `/mcp` and pass its per-agent bearer token. Once connected, every agent should use a stable `agent_name` such as `CC-Stealth` or `Codex-HSR`.

Public HTTP deployments behind a reverse proxy must also set `OPS_BRAIN_ALLOWED_HOSTS` to your hostname — see the config table below.

## Surface (15 tools)

- **Knowledge** — `add_knowledge`, `update_knowledge`, `delete_knowledge`, `search_bus`. Cross-agent gotchas, safety warnings, compliance rules, and vendor behavior, with per-agent provenance via `author`. `search_bus` searches knowledge by default and can include handoffs when requested.
- **Handoffs** — `create_handoff`, `get_handoff`, `accept_handoff`, `complete_handoff`, `list_handoffs`, `delete_handoff`, `list_replies_to_me`, `mark_merged`. `action`-class for required work; `notify`-class for FYI broadcasts (auto-pruned after 7 days). Threading via `in_reply_to`; commit linkage via `commit_hash` on completion + `mark_merged` at integration time. `get_handoff` retrieves one exact handoff without pulling unrelated queue entries.
- **Team bus** — `check_in` returns open action handoffs (pending + accepted) and recent notifications addressed to your `agent_name`.
- **Live peers (experimental)** — `list_live_peers` and `send_live_message` route untrusted text to connected Claude Code and Codex adapters. This lane is best-effort and process-local: nothing is stored or queued, and absent peers require a handoff. Packaged adapters live in [`adapters/claude-channel`](adapters/claude-channel) and [`adapters/codex-app-server`](adapters/codex-app-server). See [`docs/live-messaging.md`](docs/live-messaging.md).

Daily and weekly handoff briefings remain available as the stateless REST endpoint `POST /api/briefing`; maintenance operations such as embedding backfills stay out of every agent's MCP context.

Run embedding maintenance from an operator shell when needed:

```bash
ops-brain backfill-embeddings [--table knowledge|handoffs] [--batch-size 10]
```

## Cross-client safety

One ops-brain deployment is one trusted coordination domain. All authenticated MCP agents can reach fleet-wide handoffs, and the main bearer remains an unbound operator credential. Per-agent tokens bind identity; they are not tenant authorization.

Within that trust domain, client scoping provides a strong accidental-disclosure guard for knowledge searches:

- `client_id IS NULL` → always allowed (global)
- Same client → always allowed
- Different client + `cross_client_safe = true` → allowed (logged)
- Different client + `acknowledge_cross_client = true` → released (logged)
- Otherwise → **withheld**, replaced with a scope-mismatch notice (logged)

Every audit event lands in the `audit_log` table.

The gate is inactive when a knowledge query omits `client_slug`, so it is not a tenant-isolation or hostile-user security boundary. Deploy separate ops-brain instances when clients or operators must not be able to access one another's data.

## Stack

| Component | Choice |
|-----------|--------|
| Language | Rust 2021 |
| MCP SDK | [rmcp](https://github.com/modelcontextprotocol/rust-sdk) 1.6 |
| Database | PostgreSQL 18 |
| SQL | sqlx (async, runtime queries) |
| Async | tokio |
| Embeddings | nomic-embed-text via Ollama (768d, OpenAI-compatible API) |
| Vector index | pgvector HNSW cosine |
| Transport | stdio or HTTP (axum) |

## Configuration

| Env var | Default | Notes |
|---------|---------|-------|
| `DATABASE_URL` | (required) | PostgreSQL connection string |
| `OPS_BRAIN_TRANSPORT` | `stdio` | Transport: `stdio` or `http` |
| `OPS_BRAIN_LISTEN` | `0.0.0.0:3000` | HTTP bind address |
| `OPS_BRAIN_AUTH_TOKEN` | (none) | Bearer token for HTTP auth. Required for `http` transport — a missing or blank token aborts startup unless `OPS_BRAIN_DEV_NO_AUTH=true` explicitly opts into an open dev server. |
| `OPS_BRAIN_MACHINE_TOKENS` | (none) | JSON array of scoped, identity-bound tokens for non-interactive `POST /api/handoff` and `GET /api/pending` callers. See [`docs/machine-callers.md`](docs/machine-callers.md). |
| `OPS_BRAIN_AGENT_TOKENS` | (none) | JSON array of identity-bound tokens for interactive `/mcp` and `/live` connections. These enforce identity, not tenant isolation. See [`docs/agent-tokens.md`](docs/agent-tokens.md). |
| `OPS_BRAIN_DEV_NO_AUTH` | `false` | Explicitly serve HTTP without authentication (dev only — never expose beyond localhost) |
| `OPS_BRAIN_ALLOWED_HOSTS` | loopback only | Comma-separated allowed `Host` header values for HTTP transport (rmcp DNS-rebind mitigation). Public deploys behind a reverse proxy must set their hostname. |
| `OPS_BRAIN_MIGRATE` | `true` | Run migrations on startup |
| `OPS_BRAIN_EMBEDDINGS_ENABLED` | `true` | Set `false` to disable embeddings |
| `OPS_BRAIN_EMBEDDING_URL` | `http://localhost:11434/v1/embeddings` | OpenAI-compatible embedding API |
| `OPS_BRAIN_EMBEDDING_MODEL` | `nomic-embed-text` | Embedding model name |
| `OPS_BRAIN_EMBEDDING_API_KEY` | (none) | Bearer for the embedding API, if needed |

The configured embedding endpoint receives knowledge/handoff text during
writes and search queries in semantic or hybrid mode. Use a local endpoint or
disable embeddings when that content must not leave the deployment's trust
boundary.

Recommended agent names mirror the CC fleet convention: `CC-Stealth`, `Codex-Stealth`, `Codex-HSR`, etc. Names are still free-form slugs for compatibility; ops-brain stores exactly what the caller sends.

## Fleet stewardship

Claude Code and Codex each have their own adapter ergonomics, but ops-brain primitives stay fleet-neutral. Family-specific channel/App Server behavior belongs in the local adapter, not in server-side `cc_*` or `codex_*` branches.

## HTTP endpoints

```
POST /api/handoff   machine token with `create` scope
GET  /api/pending   machine token with `read` scope
POST /api/briefing  main bearer; `{ "type": "daily" | "weekly" }`
GET  /health        unauthenticated liveness probe
GET  /ready         unauthenticated database-readiness probe
GET  /live          agent token; ephemeral WebSocket adapter transport
```

Bearer auth protects `/mcp`, `/live`, and the three `/api` endpoints. Agent tokens can reach `/mcp` and `/live`; machine tokens remain restricted to their documented REST endpoints. `/health` and `/ready` intentionally require no bearer so container healthchecks and reverse proxies can distinguish a running process from a database-ready service.

Production compose does not publish port 3000 on the host; the service is reached through the Docker networks and the reverse proxy. For local production-host checks, run health probes inside the container or use the public reverse-proxy URL.

## Status

ops-brain is designed for solo operators and small trusted teams coordinating Claude Code and Codex across hosts. The working core remains durable handoffs, bounded shared knowledge, check-in, and narrow REST endpoints; the 15-tool development surface also contains an experimental online-only live lane with packaged host adapters.
