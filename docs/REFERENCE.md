# ops-brain Reference

On-demand reference for ops-brain internals. Read this when you need specific details
about subsystems, environment variables, or project layout.

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
    mod.rs           # OpsBrain struct + #[tool] stubs (delegate to category modules)
    helpers.rs       # Shared helpers: json_result, error_result, not_found_with_suggestions, filter_cross_client, compact_*, etc.
    shared.rs        # Shared async functions: embed_and_store, get_query_embedding, build_client_lookup, log_audit_entries
    inventory.rs     # Parameter structs + handler implementations for inventory tools (23 tools)
    runbooks.rs      # Parameter structs + handler implementations for runbook tools
    knowledge.rs     # Parameter structs + handler implementations for knowledge tools
    context.rs       # Parameter/response structs + handler implementations for context tools
    incidents.rs     # Parameter structs + handler implementations for incident tools
    coordination.rs  # Parameter structs + handler implementations for session + handoff tools
    monitoring.rs    # Parameter structs + handler implementations for monitoring tools
    search.rs        # Parameter structs + handler implementations for semantic search tools
    zammad.rs        # Parameter structs + handler implementations for Zammad ticketing tools
    briefings.rs     # Parameter/response structs + handler implementations for briefing tools
  embeddings.rs    # OpenAI embedding client + text preparation functions
  metrics.rs       # Uptime Kuma /metrics scraper (Prometheus format parser)
  watchdog.rs      # Proactive monitoring: polls Kuma, detects transitions, auto-creates incidents
  zammad.rs        # Zammad REST API client (HTTP, Token auth, ticket/article CRUD)
migrations/        # 39 sqlx migration files (auto-run on startup)
seed/seed.sql      # Idempotent seed data (clients, sites, networks)
```

## Development

```sh
just db-up          # Start local PostgreSQL (Docker)
just run            # Build + run (auto-migrates)
just watch          # Auto-reload on changes
just check          # fmt + clippy + test
```

### Build Tooling

- **Linker**: mold via `.cargo/config.toml` — incremental dev builds ~2s with hot cache
- **Migrations**: sqlx-cli — `cargo install sqlx-cli --features postgres --no-default-features`
- **Dev commands**: just — see `justfile` for all recipes
- **File watcher**: watchexec — used by `just watch`

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | (required) | PostgreSQL connection string |
| `OPS_BRAIN_TRANSPORT` | `stdio` | Transport: `stdio` or `http` |
| `OPS_BRAIN_LISTEN` | `0.0.0.0:3000` | HTTP bind address |
| `OPS_BRAIN_AUTH_TOKEN` | (none) | Bearer token for HTTP auth |
| `OPS_BRAIN_MIGRATE` | `true` | Run migrations on startup |
| `UPTIME_KUMA_URL` | (none) | Uptime Kuma base URL for /metrics scraping (single instance) |
| `UPTIME_KUMA_USERNAME` | (none) | Basic auth username for /metrics (single instance) |
| `UPTIME_KUMA_PASSWORD` | (none) | Basic auth password for /metrics (single instance) |
| `UPTIME_KUMA_INSTANCES` | (none) | Multiple Kuma instances as JSON array (takes precedence over URL). Format: `[{"name":"cloud","url":"http://..."},{"name":"lab","url":"http://..."}]` |
| `OPS_BRAIN_EMBEDDING_URL` | `http://localhost:11434/v1/embeddings` | Embedding API URL (OpenAI-compatible) |
| `OPS_BRAIN_EMBEDDING_MODEL` | `nomic-embed-text` | Embedding model name |
| `OPS_BRAIN_EMBEDDING_API_KEY` | (none) | API key for embedding service (not needed for ollama) |
| `OPS_BRAIN_EMBEDDINGS_ENABLED` | `true` | Set to `false` to disable embeddings entirely |
| `OPS_BRAIN_WATCHDOG_ENABLED` | `false` | Enable proactive monitoring watchdog |
| `OPS_BRAIN_WATCHDOG_INTERVAL` | `60` | Watchdog polling interval in seconds |
| `OPS_BRAIN_WATCHDOG_CONFIRM_POLLS` | `3` | Consecutive DOWN polls before creating incident (flap suppression) |
| `OPS_BRAIN_WATCHDOG_COOLDOWN_SECS` | `1800` | Seconds after resolving before creating new incident for same monitor |
| `OPS_BRAIN_WATCHDOG_FLAP_THRESHOLD` | `5` | Global chronic flapper threshold (per-monitor overrides via `link_monitor`) |
| `ZAMMAD_URL` | (none) | Zammad API base URL |
| `ZAMMAD_API_TOKEN` | (none) | Zammad API token for authentication |
| `ZAMMAD_DEFAULT_OWNER_ID` | (none) | Default Zammad user ID for ticket assignment (omit to leave unassigned) |
| `RUST_LOG` | `ops_brain=info` | Tracing filter |

## Semantic Search

- **Extensions**: pgvector (HNSW indexes, cosine distance), pg_trgm (trigram similarity for fuzzy slug matching)
- **Embeddings**: ollama nomic-embed-text (768 dims) via OpenAI-compatible API, nullable column
- **Search**: Hybrid RRF (Reciprocal Rank Fusion) combines FTS rank + vector similarity. Per-method candidate pool: 50 (FTS top 50 + vector top 50 -> RRF merge -> final limit).
- **FTS query parsing**: `websearch_to_tsquery` (supports quoted phrases `"exact match"`, `or` keyword, `-exclusion`). When AND returns zero results and query has 2+ words, automatically retries with OR-joined terms via `to_tsquery` (`build_or_tsquery_text()` helper in `repo/mod.rs`). Applied to all FTS-only paths (standalone search + hybrid fallback branches). `search_inventory` uses websearch_to_tsquery but not OR fallback (broad discovery tool).
- **Title boosting**: Embedding text preparation (`src/embeddings.rs`) repeats the title to give vector search stronger weight on title-matching queries. FTS already weights title as category A (highest) in stored tsvectors.
- **Tables**: runbooks, knowledge, incidents, handoffs
- **Auto-embed**: create/update tools generate embeddings best-effort (graceful on failure)
- **Truncation**: `truncate_for_embedding()` caps text at 6K chars (~5200-6000 tokens). Real markdown/code tokenizes at ~1 char/token for nomic-embed-text -- do NOT increase this limit without empirical testing.
- **Backfill**: `backfill_embeddings` tool for existing data without embeddings. Must run after title-boost change to regenerate vectors.
- **Graceful degradation**: If embedding API unreachable, all FTS works unchanged. `search_knowledge` with hybrid mode falls back to FTS-only (with OR relaxation).
- **`semantic_search` merged into `search_knowledge`**: Use `tables` param to search across multiple tables. Default is `["knowledge"]`; set `tables=["knowledge","runbooks","incidents","handoffs"]` for cross-table search. Mode defaults to `"hybrid"` for multi-table.
- **Context enrichment**: `get_situational_awareness` and `get_server_context` use vector search to find related runbooks/knowledge beyond explicit links
- **pgvector crate**: `pgvector 0.4` with `sqlx` feature for `Vector` type

## Watchdog

- **Module**: `src/watchdog.rs` -- background tokio task, no new dependencies
- **Enable**: `OPS_BRAIN_WATCHDOG_ENABLED=true` + at least one Uptime Kuma instance configured
- **Instances**: Supports multiple Uptime Kuma instances via `UPTIME_KUMA_INSTANCES` JSON env var. Falls back to single `UPTIME_KUMA_URL` for backward compat.
- **Multi-instance naming**: When >1 instance is configured, monitor names are prefixed with `instance_name/` (e.g. `linux-lab/DC Ping`). Single instance = no prefix (backward compat). All lookups are prefix-tolerant: try exact match first, then strip the `instance/` prefix as fallback.
- **Interval**: `OPS_BRAIN_WATCHDOG_INTERVAL=60` (seconds, default 60)
- **Behavior**: Polls all Kuma instances via `fetch_all_metrics()` every interval. Tracks states in memory. Detects UP->DOWN (auto-creates incident with severity from server roles, symptoms from monitor data, linked server/service, suggested runbooks). Detects DOWN->UP (auto-resolves with TTR). On startup, recovers state from open `[AUTO]` incidents.
- **Noise reduction**: Grace period (CONFIRM_POLLS, default 3), Cooldown (COOLDOWN_SECS, default 1800), Deduplication (reopens recently resolved incidents within 24h instead of creating duplicates).
- **Severity logic**: monitor `severity_override` (if set via `link_monitor`) -> server roles -> default "medium"

## Zammad Integration

- **Module**: `src/zammad.rs` -- HTTP client for Zammad REST API
- **Auth**: `Token token={api_token}` header (NOT Bearer)
- **Always uses `?expand=true`** for human-readable responses
- **Client mapping**: `clients` table has `zammad_org_id`, `zammad_group_id`, `zammad_customer_id` columns
- **Ticket links**: `ticket_links` table maps Zammad ticket IDs to ops-brain incidents/servers/services
- **State IDs**: new=1, open=2, pending_reminder=3, closed=4
- **Priority IDs**: low=1, normal=2, high=3
- **Context enrichment**: `get_client_overview` shows recent tickets, `get_situational_awareness` and `get_server_context` show linked tickets

## REST API

- **Endpoint**: `POST /api/briefing`
- **Auth**: Same bearer token as MCP (`Authorization: Bearer <token>`)
- **Body**: `{"type": "daily"|"weekly", "client_slug": null|"<slug>"}`
- **Response**: JSON with structured briefing data + markdown content + briefing_id
- **Purpose**: Enables external consumers (scheduled triggers, cron, webhooks) without MCP protocol

## CI Pipeline

GitHub Actions runs on every push to `main` and every PR. Two jobs:

1. **check** -- Format + Lint + Test (PostgreSQL 18 + pgvector service container)
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test` (unit + integration -- migrations auto-run, seed data NOT loaded)
2. **audit** -- `cargo-audit` for known vulnerabilities in dependencies

CI must pass before merging. Fix locally with `just check` before pushing.
