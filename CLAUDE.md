# ops-brain

Rust MCP server providing operational intelligence for IT infrastructure management.

## Quick Reference

- **Language**: Rust 2021 edition
- **MCP SDK**: rmcp 1.2 (`#[tool_router]` macro pattern)
- **Database**: PostgreSQL 18 via sqlx (runtime queries, not compile-time checked)
- **Transport**: stdio (local) or streamable HTTP (remote, via axum)
- **Binary**: `target/release/ops-brain` (11MB)

## Project Layout

```
src/
  main.rs          # Entry point: config, DB pool, migrations, stdio/http transport
  config.rs        # CLI/env config via clap
  db.rs            # PgPool creation + migration runner
  auth.rs          # Bearer token validation middleware (axum)
  models/          # Domain structs (sqlx::FromRow + serde derives)
  repo/            # Database query layer (all runtime query_as, not macros)
  tools/
    mod.rs           # OpsBrain struct with ALL 47 #[tool] methods in one impl block
    inventory.rs     # Parameter structs for inventory tools
    runbooks.rs      # Parameter structs for runbook tools
    knowledge.rs     # Parameter structs for knowledge tools
    context.rs       # Parameter structs + response structs for context tools
    incidents.rs     # Parameter structs for incident tools
    coordination.rs  # Parameter structs for session + handoff tools
    monitoring.rs    # Parameter structs for monitoring tools
    search.rs        # Parameter structs for semantic search tools
  embeddings.rs    # OpenAI embedding client + text preparation functions
  metrics.rs       # Uptime Kuma /metrics scraper (Prometheus format parser)
migrations/        # 16 sqlx migration files (auto-run on startup)
seed/seed.sql      # Idempotent seed data with real infrastructure
```

## Architecture Constraints

- All `#[tool]` methods MUST be in the single `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs` — rmcp macro requirement
- Parameter structs go in sub-modules (inventory.rs, etc.) and are referenced from tool methods
- Tool errors return `Ok(CallToolResult::error(...))`, never `Err(McpError)`
- Slugs are the public API (not UUIDs) — tools resolve slugs to IDs internally
- Tracing writes to stderr (critical: stdout is the MCP stdio transport)
- IDs use UUIDv7 (`Uuid::now_v7()`) for time-ordered sorting
- FTS uses PostgreSQL tsvector with weighted columns + GIN indexes
- Semantic search uses pgvector (HNSW cosine) + ollama nomic-embed-text (768 dims)
- Embedding API is OpenAI-compatible (works with ollama, OpenAI, or any compatible provider)
- Embedding column is nullable — records work fine without embeddings
- Hybrid search uses Reciprocal Rank Fusion (RRF) to combine FTS + vector results

## Development

```sh
# Prerequisites: PostgreSQL 18 running locally
just db-up          # Start local PostgreSQL (Docker) — OR use system PostgreSQL
just run            # Build + run (auto-migrates)
just watch          # Auto-reload on changes
just check          # fmt + clippy + test

# Manual seed (if using system PostgreSQL):
psql -U ops_brain -d ops_brain -f seed/seed.sql
```

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | (required) | PostgreSQL connection string |
| `OPS_BRAIN_TRANSPORT` | `stdio` | Transport: `stdio` or `http` |
| `OPS_BRAIN_LISTEN` | `0.0.0.0:3000` | HTTP bind address |
| `OPS_BRAIN_AUTH_TOKEN` | (none) | Bearer token for HTTP auth |
| `OPS_BRAIN_MIGRATE` | `true` | Run migrations on startup |
| `UPTIME_KUMA_URL` | (none) | Uptime Kuma base URL for /metrics scraping |
| `UPTIME_KUMA_USERNAME` | (none) | Basic auth username for /metrics (if needed) |
| `UPTIME_KUMA_PASSWORD` | (none) | Basic auth password for /metrics (if needed) |
| `OPS_BRAIN_EMBEDDING_URL` | `http://localhost:11434/v1/embeddings` | Embedding API URL (OpenAI-compatible) |
| `OPS_BRAIN_EMBEDDING_MODEL` | `nomic-embed-text` | Embedding model name |
| `OPS_BRAIN_EMBEDDING_API_KEY` | (none) | API key for embedding service (not needed for ollama) |
| `OPS_BRAIN_EMBEDDINGS_ENABLED` | `true` | Set to `false` to disable embeddings entirely |
| `RUST_LOG` | `ops_brain=info` | Tracing filter |

## Phase Status

- **Phase 1** (local dev): COMPLETE — 26 tools, stdio transport, verified working
- **Phase 2** (remote deploy): COMPLETE — HTTP transport + auth, deployed to kensai.cloud
- **Phase 3** (incidents + coordination): COMPLETE — 14 new tools (6 incident, 3 session, 5 handoff), 40 total
- **Phase 4** (monitoring integration): COMPLETE & DEPLOYED — 5 new tools (list_monitors, get_monitor_status, get_monitoring_summary, link_monitor, unlink_monitor), 45 total. On-demand /metrics scraping from Uptime Kuma. Monitor-to-server/service mapping. Context tools enriched with live monitoring data. All 32 monitors linked. Uptime Kuma admin creds configured in production .env.
- **Phase 5** (semantic search): COMPLETE & DEPLOYED — 2 new tools (semantic_search, backfill_embeddings), 47 total. pgvector + ollama nomic-embed-text (768 dims). Hybrid RRF search (FTS + vector). Existing search tools enhanced with `mode` param (fts/semantic/hybrid). Context tools enriched with semantically related runbooks/knowledge. Auto-embed on create/update, backfill tool for existing data. All seed data backfilled (local + remote).
- **Phase 6** (proactive monitoring): PLANNED — Scheduled watchdog that polls Uptime Kuma, detects status changes (UP→DOWN transitions), auto-creates incidents with linked servers/services, surfaces relevant runbooks, and sends notifications. Turns ops-brain from reactive query tool into an active monitoring system.
- **Phase 7** (Zammad integration): PLANNED — Connect to Zammad ticketing on kensai.cloud. Link tickets to ops-brain entities, enrich ticket context with situational awareness.
- **Phase 8** (scheduled briefings): PLANNED — Daily/weekly operational summaries. Open incidents, monitoring health, pending handoffs, recent changes. Delivered via Claude Code scheduled triggers or email.

## Deployment (kensai.cloud)

- **URL**: `https://ops.kensai.cloud/mcp`
- **Stack**: Docker on kensai.cloud behind Caddy + Cloudflare Tunnel
- **Database**: shared-postgres (same as Zammad, Nextcloud)
- **Compose**: `docker-compose.prod.yml` — uses `traefik-net` + `shared-db` networks
- **Auth**: Bearer token in `OPS_BRAIN_AUTH_TOKEN` env var
- **Health**: `GET /health` (unauthenticated, used by Docker healthcheck)

## Monitoring (Uptime Kuma)

- **URL**: `https://uptime.kensai.cloud` (v2.2.1)
- **32 monitors**: 8 push (ops scripts), 6 HTTP (web services), 1 TCP (SSH), 17 Docker containers
- **Push integration**: Ops scripts in `~/ops/` push heartbeats via cron; URLs in `~/ops/conf/.env`
- **Admin creds**: `~/docker/uptime-kuma/.env` on kensai.cloud
- **v2 API quirks**:
  - socket.io only — no REST except `/api/push/:token` and `/metrics` (Prometheus)
  - Two-phase setup: `POST /setup-database` first, then socket.io for everything else
  - `add` event (not `addMonitor`), requires `conditions` field (can be `[]`) and `notificationIDList` (can be `[]`)
  - Push tokens are client-generated, not auto-assigned
- **Integration**: ops-brain scrapes `/metrics` on demand (no polling). Monitor mappings stored in `monitors` table. `/metrics` requires basic auth (admin creds).
- **Internal URL**: `http://uptime-kuma:3001` (on `traefik-net` Docker network)
- **Tools**: `list_monitors`, `get_monitor_status`, `get_monitoring_summary`, `link_monitor`, `unlink_monitor`
- **Context enrichment**: `get_situational_awareness` and `get_server_context` include live monitoring for linked monitors

## Key Tool: get_situational_awareness

The most important tool. Accepts `server_slug`, `service_slug`, or `client_slug` and returns a comprehensive briefing: entity details, related entities, services, networks, recent incidents, relevant runbooks, vendor contacts, pending handoffs, knowledge entries, live monitoring status (if Uptime Kuma configured), and semantically related content (if embeddings configured).

## Semantic Search

- **Extension**: pgvector (HNSW indexes, cosine distance)
- **Embeddings**: ollama nomic-embed-text (768 dims) via OpenAI-compatible API, nullable column
- **Search**: Hybrid RRF (Reciprocal Rank Fusion) combines FTS rank + vector similarity
- **Tables**: runbooks, knowledge, incidents, handoffs
- **Auto-embed**: create/update tools generate embeddings best-effort (graceful on failure)
- **Backfill**: `backfill_embeddings` tool for existing data without embeddings
- **Graceful degradation**: If embedding API unreachable, all FTS works unchanged. semantic_search falls back to FTS-only.
- **Context enrichment**: `get_situational_awareness` and `get_server_context` use vector search to find related runbooks/knowledge beyond explicit links
- **pgvector crate**: `pgvector 0.4` with `sqlx` feature for `Vector` type
- **Local**: ollama service on stealth (RTX 3070, GPU-accelerated)
- **Remote**: ollama container on kensai.cloud (CPU-only, same Docker network as ops-brain)
