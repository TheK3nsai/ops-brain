# ops-brain

Rust MCP server providing operational intelligence for IT infrastructure management.

## Quick Reference

- **Language**: Rust 2021 edition
- **MCP SDK**: rmcp 1.2 (`#[tool_router]` macro pattern)
- **Database**: PostgreSQL 18 via sqlx (runtime queries, not compile-time checked)
- **Transport**: stdio (local) or streamable HTTP (remote, via axum)
- **REST API**: `POST /api/briefing` — same bearer auth, no MCP protocol needed
- **Binary**: `target/release/ops-brain`

## Project Layout

```
src/
  main.rs          # Entry point: config, DB pool, migrations, stdio/http transport
  api.rs           # REST API handlers (POST /api/briefing) + shared briefing generation logic
  config.rs        # CLI/env config via clap
  db.rs            # PgPool creation + migration runner
  auth.rs          # Bearer token validation middleware (axum)
  models/          # Domain structs (sqlx::FromRow + serde derives)
  repo/            # Database query layer (all runtime query_as, not macros)
  tools/
    mod.rs           # OpsBrain struct with ALL 59 #[tool] methods in one impl block
    inventory.rs     # Parameter structs for inventory tools
    runbooks.rs      # Parameter structs for runbook tools
    knowledge.rs     # Parameter structs for knowledge tools
    context.rs       # Parameter structs + response structs for context tools
    incidents.rs     # Parameter structs for incident tools
    coordination.rs  # Parameter structs for session + handoff tools
    monitoring.rs    # Parameter structs for monitoring tools
    search.rs        # Parameter structs for semantic search tools
    zammad.rs        # Parameter structs for Zammad ticketing tools
    briefings.rs     # Parameter structs + response structs for briefing tools
  embeddings.rs    # OpenAI embedding client + text preparation functions
  metrics.rs       # Uptime Kuma /metrics scraper (Prometheus format parser)
  watchdog.rs      # Proactive monitoring: polls Kuma, detects transitions, auto-creates incidents
  zammad.rs        # Zammad REST API client (HTTP, Token auth, ticket/article CRUD)
migrations/        # 23 sqlx migration files (auto-run on startup)
seed/seed.sql      # Idempotent seed data with real infrastructure
```

## Architecture Constraints

- All `#[tool]` methods MUST be in the single `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs` — rmcp macro requirement
- Parameter structs go in sub-modules (inventory.rs, runbooks.rs, zammad.rs, etc.) and are referenced from tool methods
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
| `OPS_BRAIN_WATCHDOG_ENABLED` | `false` | Enable proactive monitoring watchdog |
| `OPS_BRAIN_WATCHDOG_INTERVAL` | `60` | Watchdog polling interval in seconds |
| `ZAMMAD_URL` | (none) | Zammad API base URL (e.g. `http://zammad-railsserver:3000`) |
| `ZAMMAD_API_TOKEN` | (none) | Zammad API token for authentication |
| `RUST_LOG` | `ops_brain=info` | Tracing filter |

## Phase Status

- **Phase 1** (local dev): COMPLETE — 26 tools, stdio transport, verified working
- **Phase 2** (remote deploy): COMPLETE — HTTP transport + auth, deployed to kensai.cloud
- **Phase 3** (incidents + coordination): COMPLETE — 14 new tools (6 incident, 3 session, 5 handoff), 40 total
- **Phase 4** (monitoring integration): COMPLETE & DEPLOYED — 5 new tools (list_monitors, get_monitor_status, get_monitoring_summary, link_monitor, unlink_monitor), 45 total. On-demand /metrics scraping from Uptime Kuma. Monitor-to-server/service mapping. Context tools enriched with live monitoring data. All 32 monitors linked. Uptime Kuma admin creds configured in production .env.
- **Phase 5** (semantic search): COMPLETE & DEPLOYED — 2 new tools (semantic_search, backfill_embeddings), 47 total. pgvector + ollama nomic-embed-text (768 dims). Hybrid RRF search (FTS + vector). Existing search tools enhanced with `mode` param (fts/semantic/hybrid). Context tools enriched with semantically related runbooks/knowledge. Auto-embed on create/update, backfill tool for existing data. All seed data backfilled (local + remote).
- **Phase 6** (proactive monitoring): COMPLETE & DEPLOYED — Background watchdog task polls Uptime Kuma on configurable interval, detects UP→DOWN/DOWN→UP transitions, auto-creates incidents with linked servers/services/runbooks, auto-resolves on recovery with TTR. Severity auto-determined from server roles. State recovery on restart (finds open watchdog incidents). New tool: `list_watchdog_incidents`. Env: `OPS_BRAIN_WATCHDOG_ENABLED=true`, `OPS_BRAIN_WATCHDOG_INTERVAL=60`.
- **Phase 7** (Zammad integration): COMPLETE — 8 new tools (list_tickets, get_ticket, create_ticket, update_ticket, add_ticket_note, search_tickets, link_ticket, unlink_ticket), 56 total. Live Zammad REST API queries via Token auth. Client mapping (zammad_org_id/group_id/customer_id on clients table). ticket_links table for linking tickets to incidents/servers/services. Context tools enriched with ticket data (get_client_overview shows recent tickets, get_situational_awareness and get_server_context show linked tickets). Env: `ZAMMAD_URL`, `ZAMMAD_API_TOKEN`.
- **Phase 8** (scheduled briefings): COMPLETE & DEPLOYED — 3 new tools (generate_briefing, list_briefings, get_briefing), 59 total. REST API at `POST /api/briefing` (shared logic in `src/api.rs`). Aggregates monitoring health, open incidents (by severity), watchdog alerts, pending handoffs, and Zammad ticket activity into structured markdown summaries. Daily and weekly types. Weekly includes resolved incident stats (count, avg TTR) and watchdog auto-resolved count. Briefings stored in `briefings` table for historical review. Scheduled triggers deliver via Gmail: daily at 6 AM PR, weekly Monday 6 AM PR.
- **Phase 9** (client-scope safety): COMPLETE — `cross_client_safe` + `client_id` on runbooks/knowledge, `acknowledge_cross_client` gate on search/context tools, `audit_log` table, provenance injection, watchdog client-scoping. 59 tools (no new tools — safety built into existing tools).

## Safety Design Principles (Phase 9 — Implemented)

These principles govern how ops-brain handles multi-client data. The system serves a solo operator
managing two clients (HSR hospice + CPA firm) with different compliance domains (HIPAA vs IRS/tax).
Since there is no second pair of eyes, the system itself must act as the safety gate.

1. **Default-deny cross-client surfacing**: `cross_client_safe` boolean (default: false) on runbooks and knowledge tables. Content scoped to client A does NOT surface in client B context unless explicitly marked safe. The entries you forget to tag are the ones with compliance implications.

2. **Withhold-by-default on scope mismatch**: When semantic search or context tools would surface cross-client content, the actual content is **withheld** and replaced with a scope mismatch notice. An explicit `acknowledge_cross_client: true` parameter on a second call releases the result. Content that never reaches the context window can't influence reasoning. A gate, not a banner.

3. **Provenance in all results**: Every surfaced runbook and knowledge entry includes `_client_slug` and `_client_name` provenance fields. Global content (no client_id) shows `_client_name: "Global"`.

4. **Audit trail**: `audit_log` table records every cross-client surfacing attempt (withheld/released/released_safe) with tool_name, requesting_client_id, entity_type, entity_id, owning_client_id, and timestamp.

5. **Friction is a feature**: The system was built to reduce friction and enable fast context-switching. Safety friction (the acknowledgment gate) is the one place where slowing down pays for itself.

### Cross-Client Gate Behavior

- `client_id IS NULL` → always allowed (global content)
- Same client as requesting → always allowed
- Different client + `cross_client_safe = true` → allowed (marked safe)
- Different client + `cross_client_safe = false` + `acknowledge_cross_client = true` → released (audit logged)
- Different client + `cross_client_safe = false` + no acknowledgment → **WITHHELD** (notice returned, audit logged)

### Tools Affected by Cross-Client Gate

- `get_situational_awareness` — gates runbooks + knowledge via resolved client_id
- `get_server_context` — gates runbooks + knowledge via resolved client_id
- `search_runbooks` — optional `client_slug` + `acknowledge_cross_client` params
- `search_knowledge` — optional `client_slug` + `acknowledge_cross_client` params
- `semantic_search` — gates runbook + knowledge results (incidents/handoffs not gated)
- `list_runbooks` — optional `client_slug` filter (DB-level, shows client + global)
- `create_runbook` — optional `client_slug` + `cross_client_safe` params
- `update_runbook` — optional `cross_client_safe` param
- `add_knowledge` — optional `cross_client_safe` param
- Watchdog: runbook suggestions client-scoped (same-client + global only)

## Deployment (kensai.cloud)

- **URL**: `https://ops.kensai.cloud/mcp`
- **Stack**: Docker on kensai.cloud behind Caddy + Cloudflare Tunnel
- **Database**: shared-postgres (same as Zammad, Nextcloud)
- **Compose**: `docker-compose.prod.yml` — uses `traefik-net` + `shared-db` networks
- **Auth**: Bearer token in `OPS_BRAIN_AUTH_TOKEN` env var
- **Health**: `GET /health` (unauthenticated, used by Docker healthcheck)

### Multi-Instance Claude Code Configuration

The remote HTTP transport allows any Claude Code instance to connect. Cross-client safety is enforced
by the tools (via resolved client context), not by which machine you're on.

- **stealth** (local): stdio transport in `~/.claude.json` — runs local binary, connects to local PostgreSQL
- **All other machines** (HSR infra, CPA infra, kensai.cloud): http transport to `https://ops.kensai.cloud/mcp`

Remote config (add to `~/.claude.json` on each machine):
```json
{
  "mcpServers": {
    "ops-brain": {
      "type": "http",
      "url": "https://ops.kensai.cloud/mcp",
      "headers": {
        "Authorization": "Bearer <OPS_BRAIN_AUTH_TOKEN>"
      }
    }
  }
}
```

## REST API (Phase 8)

- **Endpoint**: `POST /api/briefing`
- **Auth**: Same bearer token as MCP (`Authorization: Bearer <token>`)
- **Body**: `{"type": "daily"|"weekly", "client_slug": null|"hsr"|"cpa"}`
- **Response**: JSON with structured briefing data + markdown content + briefing_id
- **Purpose**: Enables external consumers (scheduled triggers, cron, webhooks) without MCP protocol
- **Implementation**: `src/api.rs` — shared `generate_briefing_inner()` function used by both the MCP tool and REST handler

## Scheduled Triggers (Phase 8)

- **Daily**: `trig_017czYNWPXbfvek8kPagR3KT` — 6 AM PR (10:00 UTC) every day
- **Weekly**: `trig_01NA793waWBaxuB7LFiB8YNP` — 6 AM PR (10:00 UTC) every Monday
- **Delivery**: Sonnet agent curls `/api/briefing`, emails result via Gmail MCP to k3nsai@gmail.com
- **Manage**: https://claude.ai/code/scheduled

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

## Watchdog (Phase 6)

- **Module**: `src/watchdog.rs` — background tokio task, no new dependencies
- **Enable**: `OPS_BRAIN_WATCHDOG_ENABLED=true` + `UPTIME_KUMA_URL` must be set
- **Interval**: `OPS_BRAIN_WATCHDOG_INTERVAL=60` (seconds, default 60)
- **Behavior**:
  - Polls Uptime Kuma `/metrics` every interval
  - Tracks monitor states in memory (HashMap)
  - Detects UP→DOWN: auto-creates incident `[AUTO] Monitor DOWN: {name}` with severity from server roles, symptoms from monitor data, linked server/service from monitor mappings, suggested runbooks via semantic search
  - Detects DOWN→UP: auto-resolves the incident with TTR
  - On startup, recovers state from open `[AUTO]` incidents (survives restarts)
  - Graceful: if Kuma unreachable or embedding API down, logs error and continues
- **Severity logic**: domain-controller/dns/dhcp roles → critical; file-server/rds/database/backup → high; everything else → medium
- **Tool**: `list_watchdog_incidents` — query auto-created incidents by status

## Zammad Integration (Phase 7)

- **Module**: `src/zammad.rs` — HTTP client for Zammad REST API
- **Enable**: Set `ZAMMAD_URL` and `ZAMMAD_API_TOKEN`
- **Auth**: `Token token={api_token}` header (NOT Bearer)
- **Always uses `?expand=true`** for human-readable responses (state/priority/owner as names, not IDs)
- **Client mapping**: `clients` table has `zammad_org_id`, `zammad_group_id`, `zammad_customer_id` columns
- **Ticket links**: `ticket_links` table maps Zammad ticket IDs to ops-brain incidents/servers/services
- **State IDs**: new=1, open=2, pending_reminder=3, closed=4
- **Priority IDs**: low=1, normal=2, high=3
- **Owner (Eduardo)**: user_id=3
- **Clients**: HSR (org=2, group=2, customer=5), CPA (org=3, group=4, customer=6)
- **Time accounting types**: 1=Maintenance, 2=On-site, 3=Remote, 4=On-site/Remote
- **Tags**: soporte-usuario, infraestructura, instalacion, therefore, visitlink, office-365, redes, impresora, backup, monitoreo, configuracion, conectividad
- **Tools**: `list_tickets`, `get_ticket`, `create_ticket`, `update_ticket`, `add_ticket_note`, `search_tickets`, `link_ticket`, `unlink_ticket`
- **Context enrichment**: `get_client_overview` shows recent tickets, `get_situational_awareness` and `get_server_context` show linked tickets
- **Internal URL** (Docker): `http://zammad-railsserver:3000` via `shared-db` network
- **Public URL**: `https://tickets.kensai.cloud`

## Key Tool: get_situational_awareness

The most important tool. Accepts `server_slug`, `service_slug`, or `client_slug` and returns a comprehensive briefing: entity details, related entities, services, networks, recent incidents, relevant runbooks, vendor contacts, pending handoffs, knowledge entries, live monitoring status (if Uptime Kuma configured), semantically related content (if embeddings configured), and linked Zammad tickets (if Zammad configured).

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

## Gotchas

- **sqlx migration checksums are SHA-384** (48 bytes), not SHA-256 — if manually inserting into `_sqlx_migrations`, use `sha384sum` and `decode(..., 'hex')`
- **Never apply schema changes outside of migration files** — if you do, sqlx will try to re-run the migration and fail (e.g. "column already exists"). Fix: insert the migration record manually with the correct SHA-384 checksum: `INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES (<version>, '<desc>', now(), true, decode('<sha384>', 'hex'), 0);`
- **"connection closed: initialize request"** on manual `./target/release/ops-brain` run is normal — means no MCP client is connected via stdio, not an actual error
- **Migration count**: update the comment in this file's Project Layout section when adding new migrations
