# ops-brain

Rust MCP server for cross-agent coordination. Rust 2021, rmcp 1.6, PostgreSQL 18 via sqlx, stdio/HTTP transport.

**v3.0.0 — team bus only.** Inventory, incidents, and monitoring subsystems were removed: configuration management owns inventory, Zammad owns tickets/incidents, Uptime Kuma owns monitoring. ops-brain stays on its lane.

## Surface (20 tools)

- **Knowledge** (5): `add_knowledge`, `update_knowledge`, `delete_knowledge`, `search_knowledge`, `list_knowledge`
- **Handoffs** (8): `create_handoff` (optional `in_reply_to`), `accept_handoff`, `complete_handoff` (optional `commit_hash`), `list_handoffs`, `search_handoffs`, `delete_handoff`, `list_replies_to_me`, `mark_merged` (flip to `status=merged`, record `merge_commit` + `merged_at`)
- **Team bus** (1): `check_in` — open action handoffs (pending + accepted) + recent notify-class handoffs for `agent_name`
- **Search** (1): `backfill_embeddings`
- **Zammad** (4): `list_tickets`, `get_ticket`, `create_ticket`, `search_tickets`
- **Briefings** (1): `generate_briefing` (daily/weekly handoffs+tickets summary, optionally client-scoped)

## Architecture Constraints

- All `#[tool]` stubs MUST remain in the single `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs` — rmcp macro requirement. Each stub delegates to a `handle_*` function in the appropriate category module.
- Shared helpers in `tools/helpers.rs`; shared async functions in `tools/shared.rs`
- OpsBrain fields are `pub(crate)` so category modules can access pool, embedding_client, zammad_config
- Tool errors return `Ok(CallToolResult::error(...))`, never `Err(McpError)`
- Slugs are the public API (not UUIDs) — tools resolve slugs to IDs internally. On miss, pg_trgm suggests similar slugs via `not_found_with_suggestions()` in helpers.rs
- Tracing writes to stderr (critical: stdout is the MCP stdio transport)
- IDs use UUIDv7 (`Uuid::now_v7()`) for time-ordered sorting
- FTS uses PostgreSQL tsvector with weighted columns + GIN indexes; `websearch_to_tsquery` for query parsing; OR fallback when AND returns zero results
- Semantic search uses pgvector HNSW cosine + ollama nomic-embed-text (768 dims); embedding column is nullable
- Hybrid search uses Reciprocal Rank Fusion (RRF) to combine FTS + vector results

## Safety Design Principles

Multi-client data handling for a solo operator managing clients with different compliance domains (HIPAA healthcare vs tax/accounting). The system itself acts as the safety gate.

1. **Default-deny across clients**: cross-client surfacing requires explicit `acknowledge_cross_client: true` and is audit-logged.
2. **Withhold-by-default on scope mismatch**: when search tools would surface cross-client knowledge, content is **withheld** and replaced with a scope mismatch notice. An explicit `acknowledge_cross_client: true` parameter on a second call releases the result. A gate, not a banner.
3. **Provenance in all results**: every surfaced entry includes `_client_slug` and `_client_name`. Global content (no `client_id`) shows `_client_name: "Global"`.
4. **Audit trail**: `audit_log` table records every cross-client surfacing attempt with tool_name, requesting/owning client_id, entity details, and timestamp.

### Cross-Client Gate Behavior

- `client_id IS NULL` → always allowed (global content)
- Same client as requesting → always allowed
- Different client + `cross_client_safe = true` → allowed (marked safe)
- Different client + `cross_client_safe = false` + `acknowledge_cross_client = true` → released (audit logged)
- Different client + `cross_client_safe = false` + no acknowledgment → **WITHHELD** (notice returned, audit logged)

## Coordination

The team-bus principle and "no startup ritual" rules live in each agent's local instructions. Repo-specific coordination details:

- **Handoffs are the coordination layer** — creating a handoff IS the notification mechanism. `action`-category for things the recipient must do; `notify`-category for FYI broadcasts (auto-pruned after 7 days).
- **Product bar** — only build features that solve observed field pain, reduce missed/duplicate work across agents, make the next natural action clearer, and have a lifecycle. Reject ceremony, duplicate truth, generic wiki behavior, and scheduling/orchestration features that belong to cron/systemd/Task Scheduler/CI. Durable doctrine: ops-brain knowledge `019e0d79-3a7f-7902-86cc-db4a573c1071`.
- **Agent names** — use the CC-style fleet convention for every agent family: `CC-Stealth`, `Codex-Stealth`, `Gemini-Stealth`, `Codex-HSR`, etc. The validator remains free-form for compatibility, but new rows should keep that convention so handoffs route predictably.
- **Fleet stewardship** — CC, Codex, and Gemini agents may each improve ergonomics for their own client family, but shared ops-brain features must stay generic across all fleets. Family-specific work belongs in local agent instructions, onboarding docs, or compatibility guidance unless it exposes a reusable team-bus primitive.
- **Knowledge policy** — knowledge entries are for cross-agent gotchas, safety warnings, compliance rules, verified patterns, and vendor behavior ONLY. Every entry costs tokens across all agents. If it would fit in your own local instructions, put it there instead. If local docs are canonical, write a pointer/provenance entry, not a duplicate. `add_knowledge` requires `author` (your agent slug, e.g. `CC-Stealth` or `Codex-HSR`).
- **Default-deny across clients** — cross-client surfacing requires explicit `acknowledge_cross_client: true` and is audit-logged.

## Gotchas

- **sqlx migration checksums are SHA-384** (48 bytes), not SHA-256 — if manually inserting into `_sqlx_migrations`, use `sha384sum` and `decode(..., 'hex')`
- **Never modify existing migrations** — checksum mismatch will break deployments. If schema was applied outside migrations, insert the migration record manually with the correct SHA-384 checksum.
- **"connection closed: initialize request"** on manual `./target/release/ops-brain` run is normal — no MCP client connected
- **seed.sql is foundational only** — clients only. Never add fictional/placeholder data.
- **mold linker is local only** — `.cargo/config.toml` uses mold; Docker build uses its own linker. Cargo falls back if mold isn't installed.
- **sqlx-cli requires `DATABASE_URL`** — set in `.env` or export before running `sqlx migrate` commands
- **cargo-audit 0.22 has no config file support** — ignores via `--ignore RUSTSEC-XXXX` CLI flags. The `audit.toml` is documentation only; actual ignore is in `.github/workflows/ci.yml`.
- **nomic-embed-text tokenization** — real content tokenizes at ~1–1.15 chars/token, NOT ~4 chars/token. `MAX_EMBEDDING_CHARS` is 6,000. Do not increase without empirical testing.
- **Production deploys MUST use `-f docker-compose.prod.yml`** — prod uses `shared-postgres`, dev uses bundled postgres. Dev compose is project-namespaced as `ops-brain-dev` so a stray invocation can't clobber prod, but it can spin up isolated dev orphans.
- **New env vars need BOTH `.env` AND `docker-compose.prod.yml`** — prod compose enumerates every env var explicitly under `services.ops-brain.environment:` (no `env_file:`). Adding `FOO=...` to `.env` alone leaves the container booting without `FOO`. Always pair the binary's `std::env::var("FOO")` with a `- FOO=${FOO:-}` line in the prod compose.

## Development Workflow

- **Before committing non-trivial changes**: run `/prereview` — spawns the project reviewer agent to catch logic and safety issues the pre-commit hook can't. (The built-in `/review` is for an already-open PR.)
- **Pre-commit hook** catches fmt, clippy, and check automatically — no need to run these manually
- **After merging to main**: hand off the deploy to **CC-Cloud** (the canonical ops-brain deployer). The `/deploy` skill creates the handoff for you. SSH escape hatch is reserved for cases where CC-Cloud is unavailable AND the change is genuinely urgent; even then, **always** pass `-f docker-compose.prod.yml` (see Gotchas).
- **Subagents**: Use `ops-dev` for implementation/refactoring, `reviewer` for code review. Both are in `.claude/agents/`.

## What NOT to Do

- **Don't modify existing migrations** — checksum mismatch will break deployments
- **Don't use compile-time sqlx macros** — we use runtime queries for flexibility
- **Don't add tool stubs outside the `#[tool_router]` impl block** — rmcp requires them all in one place
- **Don't write to stdout** — it's the MCP stdio transport. Use `tracing::info!()` (stderr)
- **Don't add fictional/placeholder data to seed.sql**
- **Don't merge without CI green**
- **Don't add ops-brain features that duplicate local truth or create startup/session ceremony**
- **Don't reintroduce inventory, incidents, or monitoring tables** — those are owned by config management, Zammad, and Uptime Kuma respectively
