# ops-brain

Rust MCP server for cross-agent coordination. Rust 2021, rmcp 3.9, PostgreSQL 18 via sqlx, stdio/HTTP transport.

**Scope: a small team bus.** The MCP surface is handoffs, check-in, and a full-text search over handoffs; machine ingestion, wake polling, and stateless briefings use narrow REST endpoints. Inventory, incidents, monitoring, ticketing, and a knowledge store are out of scope by design: hard stops and rationale in `ROADMAP.md`, shipped history in `CHANGELOG.md`.

## Surface (8 tools)

Tool groups and REST endpoints: `README.md` → *Tools*. Keep this heading's count equal to the `#[tool(` stubs in `src/tools/mod.rs`; a deploy check parses it (`GOTCHAS.md`).

Identity: interactive sessions use **per-agent tokens** (`OPS_BRAIN_AGENT_TOKENS`) or optional external OAuth subject/client bindings. Both bind `from_agent` server-side and reach `/mcp` only, never REST; MCP write tools reject mismatching identity. The main bearer stays unbound as operator break-glass. Contracts: `docs/agent-tokens.md`, `docs/oauth.md`.

## Architecture Constraints

- All `#[tool]` stubs MUST remain in the single `#[tool_router] impl OpsBrain` block in `src/tools/mod.rs` — rmcp macro requirement. Each stub delegates to a `handle_*` function in the appropriate category module.
- Tool errors return `Ok(CallToolResult::error(...))`, never `Err(McpError)`
- Tool descriptions state what the tool does, when to use it and when not, each parameter, and its caveats; no worked examples or behavioral steering. Under-describing costs more than the tokens saved. Removing a tool means removing its mentions from `get_info` too — the compiler won't catch those.
- Agents are named by free-form slug; a handoff is named by its full UUID. There is no agent registry: the only roster is the operator's static token bindings.
- Tracing writes to stderr (critical: stdout is the MCP stdio transport)

## Restricted tokens

One deployment is one trust domain: every unrestricted agent can read every handoff. A token with `exclude_client_data` is narrowed in SQL through `Visibility` (`src/auth.rs`) to handoffs it sent or received. **Any new read path must thread `Visibility` through to its query**, because a post-fetch filter would short pages and leak counts. Contract: `docs/agent-tokens.md`.

## Coordination

**Workflow conventions ship in the server, not here.** Reply-in-thread, blocked-on-a-human, verify-before-comply, docs-stay-local, and action-vs-notify are stated in the MCP `instructions` string and tool descriptions (`src/tools/mod.rs` `get_info`, param docs in `src/tools/*.rs`) — the only text every agent on every host actually sees. Change a convention there; don't restate it in per-host instructions. A rule that is true of this fleet and false of every other deployment goes in `OPS_BRAIN_INSTRUCTIONS_EXTRA`, not in the built-in string. Every word is paid by every agent each session, so keep it terse.

Shared features stay generic: client-family-specific work belongs in client-side config unless it exposes a reusable team-bus primitive. Product bar and rationale: `ROADMAP.md`, `docs/bus-trust.md`, `docs/operator-notify.md`.

## Gotchas

- **sqlx migration checksums are SHA-384** (48 bytes), not SHA-256 — if manually inserting into `_sqlx_migrations`, use `sha384sum` and `decode(..., 'hex')`
- **Never modify existing migrations** — checksum mismatch will break deployments. If schema was applied outside migrations, insert the migration record manually with the correct SHA-384 checksum. To drop a table, write a new `DROP TABLE IF EXISTS` migration, and only when its data has zero retention value; when in doubt, leave the empty table.
- **"connection closed: initialize request"** on manual `./target/release/ops-brain` run is normal — no MCP client connected
- **Production deploys MUST use `-f docker-compose.prod.yml`** — prod uses `shared-postgres`, dev uses bundled postgres. Dev compose is project-namespaced as `ops-brain-dev` so a stray invocation can't clobber prod, but it can spin up isolated dev orphans.
- **Prod does not publish `localhost:3000` on the host** — check readiness inside the container (`docker compose -f docker-compose.prod.yml exec -T ops-brain curl -sf http://localhost:3000/ready`) and liveness through the reverse proxy (`/health`).
- **New env vars need BOTH `.env` AND `docker-compose.prod.yml`** — prod compose enumerates every var under `services.ops-brain.environment:` (no `env_file:`). Pair each new clap `#[arg(env = "FOO")]` in `src/config.rs` with `- FOO=${FOO:-}` there; `config::tests::prod_compose_passes_every_env_var` fails until you do, and keeps `OPS_BRAIN_DEV_NO_AUTH` out.

## Development Workflow

- **Before committing non-trivial changes**: run the `reviewer` agent (`.claude/agents/reviewer.md`) for a findings-first review.
- **Before every commit**: `cargo fmt --check`, `cargo clippy --all-targets -- -D warnings`, `cargo test`. This repo is public: enable the hooks once per clone with `git config core.hooksPath .githooks` (`commit-msg` guards messages; `pre-commit` runs gitleaks on staged changes, warning and passing when gitleaks isn't installed; this setting replaces any global gitleaks hook), and if your client doesn't run `.github/scripts/fleet_string_guard.py` as a pre-commit hook, run it yourself. CI detects strings only after publication.
- **Before publishing GitHub text**: use `python3 scripts/guarded-github-write.py METHOD repos/OWNER/REPO/ROUTE --input request.json`, or preflight the identical MCP/browser payload with `--dry-run`. Routes, required fields and limits: `GOTCHAS.md` → *Commit workflow*.
- **Releases**: the originating agent owns review, PR, green CI and merge. After merge, send **Claude-Cloud** a deployment-only handoff with the PR URL, the exact merged SHA, CI evidence, migration/config implications and verification criteria. Cloud deploys with its own skill and reports in-thread. Rollback stays with the cloud deployer. The fallback is an attended direct SSH deployment from the originating workstation, for when Cloud is unavailable AND the change is genuinely urgent; an unattended Cloud wake that defers the deploy does not count as unavailable.

## What NOT to Do

- **Don't use compile-time sqlx macros** — we use runtime queries for flexibility
- **Don't add ops-brain features that duplicate local truth or create startup/session ceremony**
- **Don't reintroduce a knowledge store, embeddings, or client scoping.** They were removed in v8.0.0 after a triage in which most entries turned out to be one host's local lessons (`ROADMAP.md` → *Dead forever*). The `knowledge`, `clients` and `audit_log` tables and the `embedding` columns remain in the schema because applied migrations cannot be edited; nothing reads them except `delete_handoff`, which writes its tombstone to `audit_log`.
- **Don't reintroduce inventory, incidents, monitoring, or ticketing tables** — inventory is owned by config management, monitoring by Uptime Kuma, and tickets/incidents by each client's own systems
