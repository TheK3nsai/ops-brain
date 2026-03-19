# ops-brain

Rust MCP server providing operational intelligence for IT infrastructure management.

## Quick Reference

- **Language**: Rust 2021 edition
- **MCP SDK**: rmcp 1.2 (`#[tool_router]` macro pattern)
- **Database**: PostgreSQL 18 via sqlx (runtime queries, not compile-time checked)
- **Transport**: stdio (Phase 1), streamable HTTP planned (Phase 2)
- **Binary**: `target/release/ops-brain` (11MB)

## Project Layout

```
src/
  main.rs          # Entry point: config, DB pool, migrations, stdio transport
  config.rs        # CLI/env config via clap
  db.rs            # PgPool creation + migration runner
  auth.rs          # Bearer token validation (Phase 2 placeholder)
  models/          # Domain structs (sqlx::FromRow + serde derives)
  repo/            # Database query layer (all runtime query_as, not macros)
  tools/
    mod.rs         # OpsBrain struct with ALL 26 #[tool] methods in one impl block
    inventory.rs   # Parameter structs for inventory tools
    runbooks.rs    # Parameter structs for runbook tools
    knowledge.rs   # Parameter structs for knowledge tools
    context.rs     # Parameter structs + response structs for context tools
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
| `OPS_BRAIN_LISTEN` | `0.0.0.0:3000` | HTTP bind address (Phase 2) |
| `OPS_BRAIN_AUTH_TOKEN` | (none) | Bearer token for HTTP auth (Phase 2) |
| `OPS_BRAIN_MIGRATE` | `true` | Run migrations on startup |
| `RUST_LOG` | `ops_brain=info` | Tracing filter |

## Phase Status

- **Phase 1** (local dev): COMPLETE — 26 tools, stdio transport, verified working
- **Phase 2** (remote deploy): NOT STARTED — Dockerfile + docker-compose.prod.yml ready
- **Phase 3** (incidents + coordination): Tables exist, tools deferred
- **Phase 4** (monitoring integration): Deferred until monitoring re-established
- **Phase 5** (semantic search): Future — pgvector embeddings

## Key Tool: get_situational_awareness

The most important tool. Accepts `server_slug`, `service_slug`, or `client_slug` and returns a comprehensive briefing: entity details, related entities, services, networks, recent incidents, relevant runbooks, vendor contacts, pending handoffs, and knowledge entries.
