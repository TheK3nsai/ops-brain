# ops-brain

The team bus. An [MCP](https://modelcontextprotocol.io/) server that gives Claude, Codex, Gemini, and other MCP-capable agents a shared coordination surface for the state that must cross sessions, machines, clients, or agent vendors — handoffs, knowledge, briefings, and Zammad ticket orchestration.

ops-brain is **not** local truth. Inventory belongs in your config management. Incidents belong in your ticketing system. Monitoring belongs in your monitoring stack. Reach for ops-brain only when you genuinely need the rest of the team.

## Surface (20 tools)

- **Knowledge** — `add_knowledge`, `update_knowledge`, `delete_knowledge`, `search_knowledge`, `list_knowledge`. Cross-agent gotchas, safety warnings, compliance rules, vendor behavior. Per-agent provenance via `author`. Default-deny across clients.
- **Handoffs** — `create_handoff`, `accept_handoff`, `complete_handoff`, `list_handoffs`, `search_handoffs`, `delete_handoff`, `list_replies_to_me`, `mark_merged`. `action`-class for required work; `notify`-class for FYI broadcasts (auto-pruned after 7 days). Threading via `in_reply_to`; commit-linkage via `commit_hash` on completion + `mark_merged` at integration time.
- **Team bus** — `check_in` returns open action handoffs (pending + accepted) and recent notifications addressed to your `agent_name`.
- **Search** — `backfill_embeddings` for the FTS+vector hybrid (PostgreSQL tsvector + pgvector HNSW + RRF fusion).
- **Zammad** — `list_tickets`, `get_ticket`, `create_ticket`, `search_tickets`. Resolves `client_slug` to Zammad group/org/customer.
- **Briefings** — `generate_briefing` produces daily/weekly markdown summaries (handoffs + tickets), optionally client-scoped, stored for history. Same logic available at `POST /api/briefing`.

## Cross-client safety

Designed for solo operators managing clients with different compliance domains (e.g. HIPAA healthcare vs tax/accounting). The system itself is the gate:

- `client_id IS NULL` → always allowed (global)
- Same client → always allowed
- Different client + `cross_client_safe = true` → allowed (logged)
- Different client + `acknowledge_cross_client = true` → released (logged)
- Otherwise → **withheld**, replaced with a scope-mismatch notice (logged)

Every audit event lands in the `audit_log` table.

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
| `OPS_BRAIN_AUTH_TOKEN` | (none) | Bearer token for HTTP auth |
| `OPS_BRAIN_ALLOWED_HOSTS` | loopback only | Comma-separated allowed `Host` header values for HTTP transport (rmcp DNS-rebind mitigation). Public deploys behind a reverse proxy must set their hostname. |
| `OPS_BRAIN_MIGRATE` | `true` | Run migrations on startup |
| `OPS_BRAIN_EMBEDDINGS_ENABLED` | `true` | Set `false` to disable embeddings |
| `OPS_BRAIN_EMBEDDING_URL` | `http://localhost:11434/v1/embeddings` | OpenAI-compatible embedding API |
| `OPS_BRAIN_EMBEDDING_MODEL` | `nomic-embed-text` | Embedding model name |
| `OPS_BRAIN_EMBEDDING_API_KEY` | (none) | Bearer for the embedding API, if needed |
| `ZAMMAD_URL` | (none) | Zammad base URL — disables Zammad tools if unset |
| `ZAMMAD_API_TOKEN` | (none) | Zammad API token |
| `ZAMMAD_DEFAULT_OWNER_ID` | (none) | Default owner ID for `create_ticket` |

Recommended agent names mirror the CC fleet convention: `CC-Stealth`, `Codex-Stealth`, `Gemini-Stealth`, `Codex-HSR`, etc. Names are still free-form slugs for compatibility; ops-brain stores exactly what the caller sends.

## Fleet stewardship

Claude Code, Codex CLI, Gemini CLI, and future agents can each have their own onboarding and ergonomics, but ops-brain features should stay fleet-neutral. Use agent-family stewardship to remove real friction for that client family; do not add `cc_*`, `codex_*`, or `gemini_*` server behavior unless the underlying primitive is useful to every fleet.

## REST endpoints

```
POST /api/briefing  { "type": "daily" | "weekly", "client_slug": "..." (optional) }
GET  /health
```

Bearer auth protects `/api` and `/mcp`. `/health` is intentionally unauthenticated so container healthchecks and reverse proxies can probe liveness without carrying the MCP bearer.

Production compose does not publish port 3000 on the host; the service is reached through the Docker networks and the reverse proxy. For local production-host checks, run health probes inside the container or use the public reverse-proxy URL.

## Status

ops-brain is designed for solo operators managing multiple clients. v3.0.0 is the post-debloat shape: handoffs, knowledge, briefings, Zammad. See `CLAUDE.md` for architecture constraints and `GOTCHAS.md` for the load-bearing footguns.
