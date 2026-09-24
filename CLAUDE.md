# ops-brain

Rust MCP server for cross-agent coordination. Rust 2021, rmcp 1.6, PostgreSQL 18 via sqlx, stdio/HTTP transport.

**Scope: a small team bus.** The MCP surface is handoffs, bounded knowledge, and check-in; machine ingestion, wake polling, and stateless briefings use narrow REST endpoints. Inventory, incidents, monitoring, and ticketing are out of scope by design (`ROADMAP.md`).

For roadmap philosophy + hard stops (what we will/won't build, and why), see `ROADMAP.md`. For shipped history, see `CHANGELOG.md`.

## Surface (15 tools)

- **Knowledge** (4): `add_knowledge`, `update_knowledge`, `delete_knowledge`, `search_bus` (knowledge by default; optional handoff search; empty/`*` browse)
- **Handoffs** (8): `create_handoff` (optional `in_reply_to`), `get_handoff` (exact full UUID), `accept_handoff`, `complete_handoff` (optional `commit_hash`), `list_handoffs`, `delete_handoff`, `list_replies_to_me`, `mark_merged` (flip to `status=merged`, record `merge_commit` + `merged_at`)
- **Team bus** (1): `check_in` — open action handoffs (pending + accepted) + recent notify-class handoffs for `agent_name`
- **Online peers** (2): `list_live_peers`, `send_live_message` — ephemeral online-only routing through the packaged Claude Channel and Codex App Server adapters; never queued or persisted.

REST-only (no MCP tools, zero agent token cost): `POST /api/handoff`, `GET /api/pending`, and `POST /api/briefing`. The first two serve machine-filed handoffs and wake polling through scoped machine tokens (`OPS_BRAIN_MACHINE_TOKENS`); briefings are stateless delivery output for the main bearer. `GET /health` is liveness; `GET /ready` checks PostgreSQL readiness. Producer contract in `docs/machine-callers.md`. Recurrence/dead-man stay on producers' own schedulers — ops-brain never owns execution timing.

Operator maintenance: `ops-brain backfill-embeddings [--table knowledge|handoffs] [--batch-size N]` replaces the former agent-visible backfill tool.

Identity: interactive sessions authenticate with **per-agent tokens** (`OPS_BRAIN_AGENT_TOKENS`) that bind `from_agent` server-side — MCP write tools reject mismatching identity and `/live` uses the binding as peer provenance. Agent tokens reach `/mcp` and `/live`, never REST. The main bearer stays unbound as operator break-glass and cannot register a live peer. Contract in `docs/agent-tokens.md`.

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

One deployment is one trusted coordination domain. Per-agent tokens bind provenance, not tenant authorization; every authenticated MCP agent can read fleet-wide handoffs and mutate objects by ID. Client scoping is a knowledge disclosure guard inside that trust domain, not hostile-user isolation. Use separate deployments across real trust boundaries.

1. **Scoped queries withhold by default**: cross-client surfacing requires explicit `acknowledge_cross_client: true` and is audit-logged.
2. **Withhold-by-default on scope mismatch**: when search tools would surface cross-client knowledge, content is **withheld** and replaced with a scope mismatch notice. An explicit `acknowledge_cross_client: true` parameter on a second call releases the result. A gate, not a banner.
3. **Provenance in all results**: every surfaced entry includes `_client_slug` and `_client_name`. Global content (no `client_id`) shows `_client_name: "Global"`.
4. **Audit trail**: `audit_log` table records every cross-client surfacing attempt with tool_name, requesting/owning client_id, entity details, and timestamp.

### Cross-Client Gate Behavior

- `client_id IS NULL` → always allowed (global content)
- Same client as requesting → always allowed
- Different client + `cross_client_safe = true` → allowed (marked safe)
- Different client + `cross_client_safe = false` + `acknowledge_cross_client = true` → released (audit logged)
- Different client + `cross_client_safe = false` + no acknowledgment → **WITHHELD** (notice returned, audit logged)
- No `client_slug` on the query → gate is inert (all rows returned), but every item carries provenance and the response carries a `_note` saying the gate is off (v4.1.0)

## Coordination

**Workflow conventions ship in the server, not here.** Reply-in-thread, blocked-on-a-human, verify-before-comply, the knowledge bar, and action-vs-notify are stated in the MCP `instructions` string and tool descriptions (`src/tools/mod.rs` `get_info`, param docs in `src/tools/*.rs`) — the only text every agent on every host actually sees. Change a convention there; don't restate it in per-host instructions. Every word is paid by every agent each session, so keep it terse. Background and rationale only:

- **Notify rows are hidden, not deleted** — `notify` handoffs drop out of operational queries after 7 days; the rows stay.
- **Bus trust** — full verify-before-comply reasoning: `docs/bus-trust.md`.
- **Blocked on a human** — the operator-queue cron and mail contract: `docs/operator-notify.md`.
- **Product bar** — only build features that solve observed field pain, reduce missed/duplicate work across agents, make the next natural action clearer, and have a lifecycle. Reject ceremony, duplicate truth, generic wiki behavior, and scheduling/orchestration features that belong to cron/systemd/Task Scheduler/CI. Durable doctrine: ops-brain knowledge `019e0d79-3a7f-7902-86cc-db4a573c1071`.
- **Agent names** — use the `<Agent>-<Host>` convention: `Claude-Stealth`, `Codex-Stealth`, `Codex-HSR`, etc. The validator remains free-form for compatibility, but new rows should keep that convention so handoffs route predictably.
- **Fleet stewardship** — Claude Code and Codex agents may each improve ergonomics for their own client family, but shared ops-brain features must stay generic. Family-specific work belongs in local adapters, instructions, or compatibility guidance unless it exposes a reusable team-bus primitive.
- **Knowledge is pull-only** — nothing surfaces an entry unless a search hits it, so the store is only as good as its signal-to-noise. Infrastructure how-to belongs in a git reference repo agents clone, not here (`ROADMAP.md` → no generic wiki); the bus holds cross-agent gotchas and pointers.
- **Client-scope guard** — scoped knowledge queries withhold unsafe cross-client content unless explicitly acknowledged. Unscoped searches and handoffs are fleet-wide; one deployment is one trust domain.

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
- **Prod does not publish `localhost:3000` on the host** — `docker-compose.prod.yml` attaches `ops-brain` to Docker networks for the reverse proxy; it does not expose `ports:`. Verify readiness with `docker compose -f docker-compose.prod.yml exec -T ops-brain curl -sf http://localhost:3000/ready` and public liveness with `curl -sf https://<your-deploy-host>/health` (through the reverse proxy), not host-local `curl http://localhost:3000/health`.
- **New env vars need BOTH `.env` AND `docker-compose.prod.yml`** — prod compose enumerates every env var explicitly under `services.ops-brain.environment:` (no `env_file:`). Adding `FOO=...` to `.env` alone leaves the container booting without `FOO`. Always pair a new clap `#[arg(env = "FOO")]` in `src/config.rs` with a `- FOO=${FOO:-}` line in the prod compose.

## Development Workflow

- **Before committing non-trivial changes**: use the project `reviewer` agent for a local, findings-first review.
- **Before every commit**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`, and `.github/scripts/fleet_string_guard.py` (this repo is public; CI runs the same guard, but only after the push has already published the string). Commit messages are scanned by `.githooks/commit-msg`. Enable it once per clone with `git config core.hooksPath .githooks`; without it, messages are unguarded until CI. PR titles and bodies get no local git-side check: CI scans them after publication, and only an agent-side hook on `gh`/MCP writes stops them first. Run `fleet_string_guard.py --text <file>` on a body before `gh pr create`/`edit` when no such hook covers you.
- **After merging to main**: hand off the deploy to **Claude-Cloud** (fallback **Codex-Cloud**) on the VPS; the deploy procedure is a skill on that host, not in this repo. Handoffs must spell out the same prod-compose rule. SSH escape hatch is reserved for cases where the cloud deployer is unavailable AND the change is genuinely urgent; even then, **always** pass `-f docker-compose.prod.yml` and smoke via container `/ready` plus the deploy's public `/health` URL behind the reverse proxy (port 3000 is not published to the host in prod).
- **Subagents**: Use `ops-dev` for implementation/refactoring, `reviewer` for code review. Both are in `.claude/agents/`.

## What NOT to Do

- **Don't use compile-time sqlx macros** — we use runtime queries for flexibility
- **Don't merge without CI green**
- **Don't add ops-brain features that duplicate local truth or create startup/session ceremony**
- **Don't reintroduce inventory, incidents, monitoring, or ticketing tables** — inventory is owned by config management, monitoring by Uptime Kuma, and tickets/incidents by each client's own systems
