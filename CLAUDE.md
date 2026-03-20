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
    mod.rs           # OpsBrain struct with ALL 40 #[tool] methods in one impl block
    inventory.rs     # Parameter structs for inventory tools
    runbooks.rs      # Parameter structs for runbook tools
    knowledge.rs     # Parameter structs for knowledge tools
    context.rs       # Parameter structs + response structs for context tools
    incidents.rs     # Parameter structs for incident tools
    coordination.rs  # Parameter structs for session + handoff tools
migrations/        # 14 sqlx migration files (auto-run on startup)
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
| `RUST_LOG` | `ops_brain=info` | Tracing filter |

## Phase Status

- **Phase 1** (local dev): COMPLETE — 26 tools, stdio transport, verified working
- **Phase 2** (remote deploy): COMPLETE — HTTP transport + auth, deployed to kensai.cloud
- **Phase 3** (incidents + coordination): COMPLETE — 14 new tools (6 incident, 3 session, 5 handoff), 40 total
- **Phase 4** (monitoring integration): UNBLOCKED — Uptime Kuma v2.2.1 live at uptime.kensai.cloud (32 monitors, push heartbeats active)
- **Phase 5** (semantic search): Future — pgvector embeddings

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
- **Phase 4 integration paths**: Uptime Kuma webhooks → ops-brain endpoint, scrape `/metrics`, or read SQLite directly

## Key Tool: get_situational_awareness

The most important tool. Accepts `server_slug`, `service_slug`, or `client_slug` and returns a comprehensive briefing: entity details, related entities, services, networks, recent incidents, relevant runbooks, vendor contacts, pending handoffs, and knowledge entries.
