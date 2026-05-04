# ops-brain

Rust MCP server for IT operational intelligence. Rust 2021, rmcp 1.6, PostgreSQL 18 via sqlx, stdio/HTTP transport.

For project layout, env vars, subsystem details: see `docs/REFERENCE.md`
For adding tools, branch/commit conventions, PR checklist: see `docs/CONTRIBUTING.md`

## Architecture Constraints

- All `#[tool]` stubs MUST remain in the single `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs` -- rmcp macro requirement. Each stub delegates to a `handle_*` function in the appropriate category module.
- Parameter structs and handler implementations live together in category modules (inventory.rs, knowledge.rs, zammad.rs, etc.)
- Shared helpers in `tools/helpers.rs`; shared async functions in `tools/shared.rs`
- OpsBrain fields are `pub(crate)` so category modules can access pool, kuma_config, embedding_client, zammad_config
- Tool errors return `Ok(CallToolResult::error(...))`, never `Err(McpError)`
- Slugs are the public API (not UUIDs) -- tools resolve slugs to IDs internally. On miss, pg_trgm suggests similar slugs via `not_found_with_suggestions()` in helpers.rs
- Tracing writes to stderr (critical: stdout is the MCP stdio transport)
- IDs use UUIDv7 (`Uuid::now_v7()`) for time-ordered sorting
- FTS uses PostgreSQL tsvector with weighted columns + GIN indexes; `websearch_to_tsquery` for query parsing; OR fallback when AND returns zero results
- Semantic search uses pgvector HNSW cosine + ollama nomic-embed-text (768 dims); embedding column is nullable
- Hybrid search uses Reciprocal Rank Fusion (RRF) to combine FTS + vector results

## Safety Design Principles

Multi-client data handling for a solo operator managing clients with different compliance domains (e.g. HIPAA healthcare vs tax/accounting). The system itself acts as the safety gate.

1. **Default-deny cross-client surfacing**: `cross_client_safe` boolean (default: false) on knowledge and incidents. Content scoped to client A does NOT surface in client B context unless explicitly marked safe.

2. **Withhold-by-default on scope mismatch**: When search/context tools would surface cross-client content, actual content is **withheld** and replaced with a scope mismatch notice. An explicit `acknowledge_cross_client: true` parameter on a second call releases the result. A gate, not a banner.

3. **Provenance in all results**: Every surfaced entry includes `_client_slug` and `_client_name` provenance fields. Global content (no client_id) shows `_client_name: "Global"`.

4. **Audit trail**: `audit_log` table records every cross-client surfacing attempt with tool_name, requesting/owning client_id, entity details, and timestamp.

### Cross-Client Gate Behavior

- `client_id IS NULL` -> always allowed (global content)
- Same client as requesting -> always allowed
- Different client + `cross_client_safe = true` -> allowed (marked safe)
- Different client + `cross_client_safe = false` + `acknowledge_cross_client = true` -> released (audit logged)
- Different client + `cross_client_safe = false` + no acknowledgment -> **WITHHELD** (notice returned, audit logged)

### Tools Affected by Cross-Client Gate

`get_situational_awareness`, `get_server_context`, `search_inventory`, `search_knowledge` (including multi-table mode), `add_knowledge`, `update_knowledge`, `create_incident`, `update_incident`.

## Key Tool: get_situational_awareness

The most important tool. Accepts `server_slug`, `service_slug`, or `client_slug` and returns a comprehensive briefing. Use `compact=true` to reduce response size (~94K->~10K). Use `sections` to limit which parts are returned (e.g. `["server","services","monitoring"]`).

## Coordination

The team-bus principle and "no startup ritual" rules live in each CC's per-machine `CLAUDE.md`. Repo-specific coordination details:

- **Handoffs are the coordination layer** -- creating a handoff IS the notification mechanism. `action`-category for things the recipient must do; `notify`-category for FYI broadcasts (auto-pruned after 7 days).
- **Knowledge policy** -- knowledge entries are for gotchas, safety warnings, compliance rules, and vendor behavior ONLY. Every entry costs tokens across all CC instances. If it would fit in your own CLAUDE.md, put it there instead. `add_knowledge` requires `author_cc` (your CC name from your per-machine CLAUDE.md) and accepts an optional `source_incident_id` to link the entry back to the incident that produced it — provenance is immutable via the tool surface once set.
- **Default-deny across clients** -- cross-client surfacing requires explicit `acknowledge_cross_client: true` and is audit-logged.

## Gotchas

- **sqlx migration checksums are SHA-384** (48 bytes), not SHA-256 -- if manually inserting into `_sqlx_migrations`, use `sha384sum` and `decode(..., 'hex')`
- **Never modify existing migrations** -- checksum mismatch will break deployments. If schema was applied outside migrations, insert the migration record manually with the correct SHA-384 checksum.
- **"connection closed: initialize request"** on manual `./target/release/ops-brain` run is normal -- no MCP client connected
- **upsert_server is partial on update** -- COALESCE preserves omitted fields. On create, NOT NULL fields default to empty/false/active.
- **seed.sql is foundational only** -- clients, sites, networks. Never add fictional/placeholder data.
- **mold linker is local only** -- `.cargo/config.toml` uses mold; Docker build uses its own linker. Cargo falls back if mold isn't installed.
- **sqlx-cli requires `DATABASE_URL`** -- set in `.env` or export before running `sqlx migrate` commands
- **cargo-audit 0.22 has no config file support** -- ignores via `--ignore RUSTSEC-XXXX` CLI flags. The `audit.toml` is documentation only; actual ignore is in `.github/workflows/ci.yml`.
- **upsert_vendor matches by name (case-insensitive)** -- ON CONFLICT on `LOWER(name)` for active vendors
- **nomic-embed-text tokenization** -- real content tokenizes at ~1-1.15 chars/token, NOT ~4 chars/token. `MAX_EMBEDDING_CHARS` is 6,000. Do not increase without empirical testing.
- **`link_monitor` names in multi-instance mode** -- all lookups are prefix-tolerant (try exact, then strip `instance/` prefix), so linking with unprefixed Kuma names works fine.
- **Production deploys MUST use `-f docker-compose.prod.yml`** -- prod uses `shared-postgres`, dev uses bundled postgres. Dev compose is project-namespaced as `ops-brain-dev` (PR #33) so a stray invocation can't clobber prod, but it can spin up isolated dev orphans. Full context, history, and orphan cleanup in `feedback_compose_file_footgun.md`.
- **New env vars need BOTH `.env` AND `docker-compose.prod.yml`** -- prod compose enumerates every env var explicitly under `services.ops-brain.environment:` (no `env_file:`). Adding `FOO=...` to `.env` alone leaves the container booting without `FOO`. Always pair the binary's `std::env::var("FOO")` with a `- FOO=${FOO:-}` line in the prod compose. Caught at the rmcp 1.6 deploy (PR #47 → fix in #48); `/prereview` now greps for this.

## Development Workflow

- **Before committing non-trivial changes**: run `/prereview` — spawns the project reviewer agent to catch logic and safety issues the pre-commit hook can't. (The built-in `/review` is for an already-open PR.)
- **Pre-commit hook** catches fmt, clippy, and check automatically — no need to run these manually
- **After merging to main**: hand off the deploy to **CC-Cloud** (the canonical ops-brain deployer — they live on kensai.cloud and know the layout). The `/deploy` skill creates the handoff for you. SSH escape hatch is reserved for cases where CC-Cloud is unavailable AND the change is genuinely urgent; even then, **always** pass `-f docker-compose.prod.yml` (see Gotchas).
- **Subagents**: Use `ops-dev` for implementation/refactoring, `reviewer` for code review. Both are in `.claude/agents/`.

## What NOT to Do

- **Don't modify existing migrations** -- checksum mismatch will break deployments
- **Don't use compile-time sqlx macros** -- we use runtime queries for flexibility
- **Don't add tool stubs outside the `#[tool_router]` impl block** -- rmcp requires them all in one place
- **Don't write to stdout** -- it's the MCP stdio transport. Use `tracing::info!()` (stderr)
- **Don't add fictional/placeholder data to seed.sql**
- **Don't merge without CI green**
