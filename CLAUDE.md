# ops-brain

Rust MCP server for cross-agent coordination. Rust 2021, rmcp 1.6, PostgreSQL 18 via sqlx, stdio/HTTP transport.

**Scope: a small team bus.** The MCP surface is handoffs, bounded knowledge, and check-in; machine ingestion, wake polling, and stateless briefings use narrow REST endpoints. Inventory, incidents, monitoring, and ticketing are out of scope by design (`ROADMAP.md`).

For roadmap philosophy + hard stops (what we will/won't build, and why), see `ROADMAP.md`. For shipped history, see `CHANGELOG.md`.

## Surface (13 tools)

- **Knowledge** (4): `add_knowledge`, `update_knowledge`, `delete_knowledge`, `search_bus` (knowledge by default; optional handoff search; empty/`*` browse)
- **Handoffs** (8): `create_handoff` (optional `in_reply_to`), `get_handoff` (exact full UUID), `accept_handoff`, `complete_handoff` (optional `commit_hash`), `list_handoffs`, `delete_handoff`, `list_replies_to_me`, `mark_merged` (flip to `status=merged`, record `merge_commit` + `merged_at`)
- **Team bus** (1): `check_in` — open action handoffs (pending + accepted) + recent notify-class handoffs for `agent_name`

REST-only (no MCP tools, zero agent token cost): `POST /api/handoff`, `GET /api/pending`, and `POST /api/briefing`. The first two serve machine-filed handoffs and wake polling through scoped machine tokens (`OPS_BRAIN_MACHINE_TOKENS`); briefings are stateless delivery output for the main bearer. `GET /health` is liveness; `GET /ready` checks PostgreSQL readiness. Producer contract in `docs/machine-callers.md`. Recurrence/dead-man stay on producers' own schedulers — ops-brain never owns execution timing.

Operator maintenance: `ops-brain backfill-embeddings [--table knowledge|handoffs] [--batch-size N]` replaces the former agent-visible backfill tool.

Identity: interactive sessions authenticate with **per-agent tokens** (`OPS_BRAIN_AGENT_TOKENS`) that bind `from_agent` server-side — MCP write tools reject mismatching identity. Agent tokens reach `/mcp` only, never REST. The main bearer stays unbound as operator break-glass. Contract in `docs/agent-tokens.md`.

## Architecture Constraints

- All `#[tool]` stubs MUST remain in the single `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs` — rmcp macro requirement. Each stub delegates to a `handle_*` function in the appropriate category module.
- Shared helpers in `tools/helpers.rs`; shared async functions in `tools/shared.rs`
- OpsBrain fields are `pub(crate)` so category modules can access pool, embedding_client
- Tool errors return `Ok(CallToolResult::error(...))`, never `Err(McpError)`
- Slugs are the public API (not UUIDs) — tools resolve slugs to IDs internally. On miss, pg_trgm suggests similar slugs via `not_found_with_suggestions()` in helpers.rs
- Tracing writes to stderr (critical: stdout is the MCP stdio transport)
- IDs use UUIDv7 (`Uuid::now_v7()`) for time-ordered sorting
- FTS uses PostgreSQL tsvector with weighted columns + GIN indexes; `websearch_to_tsquery` for query parsing; OR fallback when AND returns zero results
- Semantic search uses pgvector HNSW cosine + ollama nomic-embed-text (768 dims); embedding column is nullable
- Hybrid search uses Reciprocal Rank Fusion (RRF) to combine FTS + vector results

## Client-Scope Disclosure Guard

One deployment is one trust domain; client scoping is a knowledge disclosure guard inside it, not tenant isolation. Scoped queries withhold cross-client content unless `acknowledge_cross_client: true` (audit-logged), and every result carries `_client_slug`/`_client_name` provenance. Restricted tokens (`exclude_client_data`) filter in SQL through `Visibility` (`src/auth.rs`): **any new read path must thread `Visibility` through to its query**, because a post-fetch filter would short pages and leak counts. Full gate table: `docs/client-scope.md`; token contract: `docs/agent-tokens.md`.

## Coordination

**Workflow conventions ship in the server, not here.** Reply-in-thread, blocked-on-a-human, verify-before-comply, the knowledge bar, and action-vs-notify are stated in the MCP `instructions` string and tool descriptions (`src/tools/mod.rs` `get_info`, param docs in `src/tools/*.rs`) — the only text every agent on every host actually sees. Change a convention there; don't restate it in per-host instructions. A rule that is true of this fleet and false of every other deployment goes in `OPS_BRAIN_INSTRUCTIONS_EXTRA`, not in the built-in string. Every word is paid by every agent each session, so keep it terse.

Shared features stay generic: client-family-specific work belongs in client-side config unless it exposes a reusable team-bus primitive. Product bar and rationale: `ROADMAP.md`, `docs/bus-trust.md`, `docs/operator-notify.md`.

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
- **Prod does not publish `localhost:3000` on the host** — check readiness inside the container (`docker compose -f docker-compose.prod.yml exec -T ops-brain curl -sf http://localhost:3000/ready`) and liveness through the reverse proxy (`/health`).
- **New env vars need BOTH `.env` AND `docker-compose.prod.yml`** — prod compose enumerates every var under `services.ops-brain.environment:` (no `env_file:`). Pair each new clap `#[arg(env = "FOO")]` in `src/config.rs` with `- FOO=${FOO:-}` there.

## Development Workflow

- **Before committing non-trivial changes**: run the `reviewer` agent (`.claude/agents/reviewer.md`) for a findings-first review.
- **Before every commit**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. This repo is public: enable the commit-message guard once per clone with `git config core.hooksPath .githooks`, and if your client doesn't run `.github/scripts/fleet_string_guard.py` as a pre-commit hook, run it yourself (`--text <file>` for a PR body). CI runs the same guard, but only after the push has published the string.
- **Releases**: the originating agent owns review, PR, green CI and merge. After merge, send **Claude-Cloud** (fallback **Codex-Cloud**) a deployment-only handoff with the PR URL, the exact merged SHA, CI evidence, migration/config implications and verification criteria. Cloud deploys with its own skill and reports in-thread. Direct SSH deployment from the originating workstation is an exception for when Cloud is unavailable AND the change is genuinely urgent.

## What NOT to Do

- **Don't use compile-time sqlx macros** — we use runtime queries for flexibility
- **Don't merge without CI green**
- **Don't add ops-brain features that duplicate local truth or create startup/session ceremony**
- **Don't reintroduce inventory, incidents, monitoring, or ticketing tables** — inventory is owned by config management, monitoring by Uptime Kuma, and tickets/incidents by each client's own systems
