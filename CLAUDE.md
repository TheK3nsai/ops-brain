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
    mod.rs           # OpsBrain struct + #[tool] stubs (delegate to category modules)
    helpers.rs       # Shared helpers: json_result, error_result, not_found_with_suggestions, filter_cross_client, compact_*, etc.
    shared.rs        # Shared async functions: embed_and_store, get_query_embedding, build_client_lookup, log_audit_entries
    inventory.rs     # Parameter structs + handler implementations for inventory tools (22 tools)
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
migrations/        # 35 sqlx migration files (auto-run on startup)
seed/seed.sql      # Idempotent seed data (clients, sites, networks)
```

## Architecture Constraints

- All `#[tool]` stubs MUST remain in the single `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs` — rmcp macro requirement. Each stub delegates to a `handle_*` function in the appropriate category module.
- Parameter structs and handler implementations live together in category modules (inventory.rs, runbooks.rs, zammad.rs, etc.)
- Shared helpers (json_result, filter_cross_client, compact_*) live in `tools/helpers.rs`; shared async functions (embed_and_store, get_query_embedding, etc.) live in `tools/shared.rs`
- OpsBrain fields are `pub(crate)` so category modules can access pool, kuma_config, embedding_client, zammad_config
- Tool errors return `Ok(CallToolResult::error(...))`, never `Err(McpError)`
- Slugs are the public API (not UUIDs) — tools resolve slugs to IDs internally. On miss, pg_trgm suggests similar slugs ("Did you mean: ...?") via `not_found_with_suggestions()` in helpers.rs
- Tracing writes to stderr (critical: stdout is the MCP stdio transport)
- IDs use UUIDv7 (`Uuid::now_v7()`) for time-ordered sorting
- FTS uses PostgreSQL tsvector with weighted columns + GIN indexes; `websearch_to_tsquery` for query parsing (supports quoted phrases, `or`, `-exclusion`); OR fallback when AND returns zero results
- Fuzzy slug matching uses pg_trgm (trigram similarity) + GIN indexes on slug/name columns
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

# Migration management (sqlx-cli):
sqlx migrate add <name>   # Scaffold new timestamped migration
sqlx migrate run           # Run pending migrations (standalone, no app startup)
sqlx migrate info          # Show migration status
```

### Build Tooling

- **Linker**: [mold](https://github.com/rui314/mold) via `.cargo/config.toml` — incremental dev builds ~2s with hot cache
- **Migrations**: [sqlx-cli](https://github.com/launchbadge/sqlx/tree/main/sqlx-cli) — `cargo install sqlx-cli --features postgres --no-default-features`
- **Dev commands**: [just](https://github.com/casey/just) — see `justfile` for all recipes
- **File watcher**: [watchexec](https://github.com/watchexec/watchexec) — used by `just watch`

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

## Safety Design Principles

These principles govern how ops-brain handles multi-client data. The system is designed for a solo
operator managing multiple clients with different compliance domains (e.g. HIPAA healthcare vs tax/accounting).
Since there is no second pair of eyes, the system itself acts as the safety gate.

1. **Default-deny cross-client surfacing**: `cross_client_safe` boolean (default: false) on runbooks, knowledge, and incidents tables. Content scoped to client A does NOT surface in client B context unless explicitly marked safe. The entries you forget to tag are the ones with compliance implications.

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

- `get_situational_awareness` — gates runbooks, knowledge, and incidents via resolved client_id
- `get_server_context` — gates runbooks, knowledge, and incidents via resolved client_id
- `search_inventory` — optional `client_slug` + `acknowledge_cross_client` params; gates runbooks, knowledge, and incidents (servers, services, vendors, sites, networks, handoffs ungated)
- `search_runbooks` — optional `client_slug` + `acknowledge_cross_client` params
- `search_knowledge` — optional `client_slug` + `acknowledge_cross_client` params
- `search_knowledge` (multi-table mode) — gates runbook, knowledge, and incident results (handoffs not gated — no client_id)
- `list_runbooks` — optional `client_slug` filter (DB-level, shows client + global)
- `create_runbook` — optional `client_slug` + `cross_client_safe` params
- `update_runbook` — optional `cross_client_safe` param
- `add_knowledge` — optional `cross_client_safe` param
- `update_knowledge` — optional `cross_client_safe` param
- `create_incident` — optional `cross_client_safe` param (default: false)
- `update_incident` — optional `cross_client_safe` param
- `delete_knowledge` — deletes by ID (no cross-client gate needed, operates on explicit ID)
- Watchdog: runbook suggestions client-scoped (same-client + global only)

## Key Tool: get_situational_awareness

The most important tool. Accepts `server_slug`, `service_slug`, or `client_slug` and returns a comprehensive briefing: entity details, related entities, services, networks, recent incidents, relevant runbooks, vendor contacts, pending handoffs, knowledge entries, live monitoring status (if Uptime Kuma configured), semantically related content (if embeddings configured), and linked Zammad tickets (if Zammad configured). Cross-client runbooks, knowledge, and incidents are auto-gated. Use `compact=true` to reduce response size (~94K→~10K) by stripping content/body fields. Use `sections` to limit which parts are returned (e.g. `["server","services","monitoring"]`).

## Semantic Search

- **Extensions**: pgvector (HNSW indexes, cosine distance), pg_trgm (trigram similarity for fuzzy slug matching)
- **Embeddings**: ollama nomic-embed-text (768 dims) via OpenAI-compatible API, nullable column
- **Search**: Hybrid RRF (Reciprocal Rank Fusion) combines FTS rank + vector similarity. Per-method candidate pool: 50 (FTS top 50 + vector top 50 → RRF merge → final limit).
- **FTS query parsing**: `websearch_to_tsquery` (supports quoted phrases `"exact match"`, `or` keyword, `-exclusion`). When AND returns zero results and query has 2+ words, automatically retries with OR-joined terms via `to_tsquery` (`build_or_tsquery_text()` helper in `repo/mod.rs`). Applied to all FTS-only paths (standalone search + hybrid fallback branches). `search_inventory` uses websearch_to_tsquery but not OR fallback (broad discovery tool).
- **Title boosting**: Embedding text preparation (`src/embeddings.rs`) repeats the title to give vector search stronger weight on title-matching queries. FTS already weights title as category A (highest) in stored tsvectors.
- **Tables**: runbooks, knowledge, incidents, handoffs
- **Auto-embed**: create/update tools generate embeddings best-effort (graceful on failure)
- **Truncation**: `truncate_for_embedding()` caps text at 6K chars (~5200-6000 tokens). Real markdown/code tokenizes at ~1 char/token for nomic-embed-text — do NOT increase this limit without empirical testing (see knowledge entry "nomic-embed-text Real-World Tokenization Ratio").
- **Backfill**: `backfill_embeddings` tool for existing data without embeddings. **Must run after title-boost change** to regenerate vectors.
- **Graceful degradation**: If embedding API unreachable, all FTS works unchanged. `search_knowledge` with hybrid mode falls back to FTS-only (with OR relaxation).
- **`semantic_search` merged into `search_knowledge`**: Use `tables` param to search across multiple tables. Default is `["knowledge"]`; set `tables=["knowledge","runbooks","incidents","handoffs"]` for cross-table search. Mode defaults to `"hybrid"` for multi-table.
- **Context enrichment**: `get_situational_awareness` and `get_server_context` use vector search to find related runbooks/knowledge beyond explicit links
- **pgvector crate**: `pgvector 0.4` with `sqlx` feature for `Vector` type

## Watchdog

- **Module**: `src/watchdog.rs` — background tokio task, no new dependencies
- **Enable**: `OPS_BRAIN_WATCHDOG_ENABLED=true` + at least one Uptime Kuma instance configured
- **Instances**: Supports multiple Uptime Kuma instances via `UPTIME_KUMA_INSTANCES` JSON env var. Falls back to single `UPTIME_KUMA_URL` for backward compat.
- **Multi-instance naming**: When >1 instance is configured, monitor names are prefixed with `instance_name/` (e.g. `linux-lab/DC Ping`). Single instance = no prefix (backward compat).
- **Interval**: `OPS_BRAIN_WATCHDOG_INTERVAL=60` (seconds, default 60)
- **Behavior**:
  - Polls all configured Uptime Kuma instances via `fetch_all_metrics()` every interval
  - Partial failure tolerant: if one instance is unreachable, monitors from other instances are still processed
  - Tracks monitor states in memory (HashMap)
  - Detects UP→DOWN: auto-creates incident `[AUTO] Monitor DOWN: {name}` with severity from server roles, symptoms from monitor data, linked server/service from monitor mappings, suggested runbooks via semantic search
  - Detects DOWN→UP: auto-resolves the incident with TTR
  - On startup, recovers state from open `[AUTO]` incidents (survives restarts)
  - Graceful: if Kuma unreachable or embedding API down, logs error and continues
- **Noise reduction** (three mechanisms):
  - **Grace period** (`CONFIRM_POLLS`, default 3): Monitor must be DOWN for N consecutive polls before an incident is created. With 60s interval, that's ~3 minutes. Handles push-monitor heartbeat jitter and transient blips.
  - **Cooldown** (`COOLDOWN_SECS`, default 1800): After auto-resolving an incident, no new incident for the same monitor for 30 minutes. Handles DOWN→UP→DOWN flapping.
  - **Deduplication**: Before creating a new incident, checks for a recently resolved incident (24h) with the same title. If found, reopens it and increments `recurrence_count` instead of creating a duplicate. Eliminates recurring heartbeat noise.
  - Set `CONFIRM_POLLS=1` and `COOLDOWN_SECS=0` to disable flap suppression (original behavior). Deduplication is always active.
- **Severity logic**: monitor `severity_override` (if set via `link_monitor`) → server roles (domain-controller/dns/dhcp → critical; file-server/rds/database/backup → high) → default "medium"
- **Tool**: `list_watchdog_incidents` — query auto-created incidents by status

## Zammad Integration

- **Module**: `src/zammad.rs` — HTTP client for Zammad REST API
- **Enable**: Set `ZAMMAD_URL` and `ZAMMAD_API_TOKEN`
- **Auth**: `Token token={api_token}` header (NOT Bearer)
- **Always uses `?expand=true`** for human-readable responses (state/priority/owner as names, not IDs)
- **Client mapping**: `clients` table has `zammad_org_id`, `zammad_group_id`, `zammad_customer_id` columns
- **Ticket links**: `ticket_links` table maps Zammad ticket IDs to ops-brain incidents/servers/services
- **State IDs**: new=1, open=2, pending_reminder=3, closed=4
- **Priority IDs**: low=1, normal=2, high=3
- **Default owner**: configurable via `ZAMMAD_DEFAULT_OWNER_ID` env var (omit to leave unassigned)
- **Tools**: `list_tickets`, `get_ticket`, `create_ticket`, `update_ticket`, `add_ticket_note`, `search_tickets`, `link_ticket`, `unlink_ticket`
- **Context enrichment**: `get_client_overview` shows recent tickets, `get_situational_awareness` and `get_server_context` show linked tickets

## REST API

- **Endpoint**: `POST /api/briefing`
- **Auth**: Same bearer token as MCP (`Authorization: Bearer <token>`)
- **Body**: `{"type": "daily"|"weekly", "client_slug": null|"<slug>"}`
- **Response**: JSON with structured briefing data + markdown content + briefing_id
- **Purpose**: Enables external consumers (scheduled triggers, cron, webhooks) without MCP protocol
- **Implementation**: `src/api.rs` — shared `generate_briefing_inner()` function used by both the MCP tool and REST handler

## Gotchas

- **sqlx migration checksums are SHA-384** (48 bytes), not SHA-256 — if manually inserting into `_sqlx_migrations`, use `sha384sum` and `decode(..., 'hex')`
- **Never apply schema changes outside of migration files** — if you do, sqlx will try to re-run the migration and fail (e.g. "column already exists"). Fix: insert the migration record manually with the correct SHA-384 checksum: `INSERT INTO _sqlx_migrations (version, description, installed_on, success, checksum, execution_time) VALUES (<version>, '<desc>', now(), true, decode('<sha384>', 'hex'), 0);`
- **"connection closed: initialize request"** on manual `./target/release/ops-brain` run is normal — means no MCP client is connected via stdio, not an actual error
- **Migration count**: update the comment in this file's Project Layout section when adding new migrations
- **upsert_server is partial on update** — when the server already exists (by slug), only provided fields are changed (COALESCE). Omitted fields are preserved. On create (new slug), NOT NULL fields default to empty/false/active.
- **seed.sql is foundational only** — clients, sites, networks. All other data comes from MCP tool sessions. Never add fictional/placeholder data to seed.sql.
- **mold linker is local only** — `.cargo/config.toml` uses mold for fast local builds. The Docker build uses its own linker (musl/gcc inside the container). Cargo falls back to the default linker if mold isn't installed.
- **sqlx-cli requires `DATABASE_URL`** — set it in `.env` or export it before running `sqlx migrate` commands. Same connection string the app uses.
- **cargo-audit 0.22 has no config file support** — ignores must be passed via `--ignore RUSTSEC-XXXX` CLI flags. The `audit.toml` in the repo root is documentation only. The actual ignore is in `.github/workflows/ci.yml`.
- **upsert_vendor matches by name (case-insensitive)** — ON CONFLICT on `LOWER(name)` for active vendors. Calling `upsert_vendor` with an existing vendor name updates it (COALESCE). Use `id` parameter to update a specific vendor by UUID.
- **nomic-embed-text tokenization** — real markdown/code content tokenizes at ~1-1.15 chars/token, NOT ~4 chars/token. `MAX_EMBEDDING_CHARS` in `src/embeddings.rs` is 6,000 (not 24K). Do not increase without empirical testing against production data — code-heavy content fails at ~7,200 chars, plain markdown at ~8,200 chars (8,192-token context window).

## CI Pipeline

GitHub Actions runs on every push to `main` and every PR. Two jobs:

1. **check** — Format + Lint + Test (PostgreSQL 18 + pgvector service container)
   - `cargo fmt --all -- --check`
   - `cargo clippy --all-targets -- -D warnings`
   - `cargo test` (unit + integration — migrations auto-run, seed data NOT loaded)
2. **audit** — `cargo-audit` for known vulnerabilities in dependencies

CI must pass before merging. If clippy or tests fail, fix locally with `just check` before pushing.

## Contributing

### Branch Naming

```
<type>/<short-description>
```

Types: `feat/`, `fix/`, `refactor/`, `docs/`, `chore/`

Examples: `feat/delete-server-tool`, `fix/vendor-dedup-slug`, `docs/runbook-template`

### Commit Messages

```
<type>: <imperative description>

<optional body — explain why, not what>
```

Types: `feat`, `fix`, `refactor`, `docs`, `chore`, `test`

Examples:
- `feat: add delete_server tool with cascade safety`
- `fix: handle null embedding in hybrid search`
- `chore: update pgvector to 0.5`

### How to Add a New Tool (End-to-End Recipe)

This is the most common contribution. Follow these steps in order:

**1. Migration (if schema changes needed)**

Create a new file in `migrations/` with the next sequence number:
```
migrations/YYYYMMDDHHMMSS_description.sql
```
- Use `IF NOT EXISTS` / `CREATE OR REPLACE` for idempotency
- Never modify existing migration files — checksums are SHA-384 and will break

**2. Model (if new table/columns)**

Add or update struct in `src/models/`. Must derive:
```rust
#[derive(Debug, Clone, sqlx::FromRow, serde::Serialize, serde::Deserialize)]
```

**3. Repository function**

Add to the appropriate `src/repo/*.rs`. Pattern:
```rust
pub async fn delete_thing(pool: &PgPool, id: &Uuid) -> Result<bool> {
    let result = sqlx::query("DELETE FROM things WHERE id = $1")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected() > 0)
}
```
- Always use runtime `sqlx::query` / `sqlx::query_as` (not compile-time macros)
- Return `Result<T>` with `anyhow`

**4. Parameter struct**

Add to the appropriate `src/tools/*.rs` (NOT mod.rs). Pattern:
```rust
#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteThingParams {
    /// The slug of the thing to delete
    pub slug: String,
    /// Must be true to confirm deletion (safety gate)
    pub confirm: bool,
}
```

**5. Handler function**

Add the handler implementation to the appropriate category file (e.g., `src/tools/inventory.rs`):
```rust
pub(crate) async fn handle_delete_thing(brain: &super::OpsBrain, params: DeleteThingParams) -> CallToolResult {
    // 1. Resolve slug to entity (brain.pool)
    // 2. Check for FK references (safety gate)
    // 3. Require confirm=true
    // 4. Delete
    // 5. Return success message via json_result()
}
```

**6. Tool stub**

Add a thin stub to the `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs`:
```rust
#[tool(description = "Delete a thing by slug. Requires confirm=true.")]
async fn delete_thing(&self, params: Parameters<inventory::DeleteThingParams>) -> Result<CallToolResult, McpError> {
    Ok(inventory::handle_delete_thing(self, params.0).await)
}
```
- Tool stubs MUST be in the single `#[tool_router] impl OpsBrain` block — rmcp macro requirement
- Stubs only delegate — all logic lives in the category handler
- Handler returns `CallToolResult` directly; stub wraps in `Ok()`
- Handler accesses `brain.pool`, `brain.embedding_client`, etc.

**7. Integration test**

Add to `tests/integration.rs`. Pattern:
```rust
#[tokio::test]
async fn test_delete_thing() {
    let pool = common::test_pool().await;
    // 1. Create test data
    // 2. Call delete
    // 3. Assert it's gone
    // 4. Assert FK safety gate works
}
```

**8. Update counts**

- Update tool count in `CLAUDE.md` (Quick Reference, Project Layout comment)
- Update tool count in `README.md`

### PR Checklist

Before opening a PR, verify:

- [ ] `cargo fmt --all -- --check` passes (no formatting issues)
- [ ] `cargo clippy --all-targets -- -D warnings` passes (no warnings)
- [ ] `cargo test` passes (all unit + integration tests)
- [ ] New tools have integration tests
- [ ] CLAUDE.md updated if tool count changed
- [ ] README.md updated if tool count changed
- [ ] No hardcoded credentials, URLs, or tokens
- [ ] Migration files are idempotent (`IF NOT EXISTS`, etc.)
- [ ] Cross-client safety considered: if the tool touches runbooks/knowledge, does it need `client_slug` and `acknowledge_cross_client` params?
- [ ] Handoff created to stealth for review/merge (PRs don't notify — handoffs do)

### What NOT to Do

- **Don't modify existing migrations** — checksum mismatch will break deployments
- **Don't use compile-time sqlx macros** — we use runtime queries for flexibility
- **Don't add tool stubs outside the `#[tool_router]` impl block** — rmcp requires them all in one place. Handler logic goes in category modules.
- **Don't write to stdout** — it's the MCP stdio transport. Use `tracing::info!()` (goes to stderr)
- **Don't add fictional/placeholder data to seed.sql** — only foundational structure
- **Don't merge without CI green** — the pipeline exists to protect us all
