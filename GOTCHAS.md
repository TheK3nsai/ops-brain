# Gotchas

## Database Migrations

- **Inventory and incident tables were dropped in v3.0.0.** Do not add them back. Configuration management (Terraform/Ansible/local config files) is the source of truth for inventory; Zammad is the source of truth for tickets/incidents; Uptime Kuma is the source of truth for monitoring. ops-brain stays on its lane: handoffs, knowledge, briefings, Zammad orchestration.
- **`knowledge.source_incident_id` was dropped** in the same migration. Provenance now lives entirely in the `author` column.
- **Re-applying a still-untracked migration after editing it.** If a new migration file has already run against your local dev DB and you edit the file before committing, sqlx will refuse to boot with `VersionMismatch(<version>)` because the checksum drifted. Recipe to recover: `DELETE FROM _sqlx_migrations WHERE version = <N>;` then `DROP` the indexes/columns it added; re-run. Only safe while the migration is unreleased — never do this once it has merged.

## Commit workflow

- **PreToolUse `cargo fmt --check` hook blocks the whole Bash invocation, including staging.** When you chain `git add <files> && git commit -m "..."` and the fmt hook trips, neither half runs — so untracked files you intended to include are still untracked. After `cargo fmt`, re-stage the originally-intended set explicitly (especially new migrations and other untracked files); `git add -u` alone only catches tracked-file edits. Caught once on 2026-05-12 (v3.1 PR #53), required a follow-up commit; squash-merge cleaned it up but is avoidable.

## Production deploy checks

- **Production compose does not publish port 3000 to the host.** `ops-brain` listens on `0.0.0.0:3000` inside the container and exposes `3000/tcp` to Docker networks for the reverse proxy; host-local `curl http://localhost:3000/health` is not a valid prod smoke test. Use `docker compose -f docker-compose.prod.yml exec -T ops-brain curl -sf http://localhost:3000/health` for the container health path, `curl -sf https://ops.kensai.cloud/health` for the public reverse-proxy path, or an MCP initialize/tools-list request through the reverse proxy. Caught during the first `Codex-Cloud` deploy smoke on 2026-05-12.
- **`/health` is unauthenticated on purpose.** The bearer middleware skips `/health` so Docker and reverse proxies can probe liveness without the MCP bearer. `/api` and `/mcp` remain bearer-protected.

## Compose files

- **`docker-compose.prod.yml` is mandatory for any prod-host compose action.** Two compose files live in the repo:

  | File | Purpose | DB target |
  |---|---|---|
  | `docker-compose.yml` | DEV / new-user — bundles its own postgres | `postgres` service inside dev project's own network, fresh dev pgdata |
  | `docker-compose.prod.yml` | PROD — joins `traefik-net` + `shared-db` external nets | `shared-postgres` (the real production DB) |

  On 2026-04-06 (PR #31 escape-hatch deploy), CC-Stealth ran `docker compose up -d --build ops-brain` without `-f`. The dev file declared `container_name: ops-brain` and `container_name: ops-brain-db`, so compose recreated the production container wired to a fresh empty postgres and ran migrations against zero rows. ~44 seconds of blank-DB serving. Recovery: `docker compose -f docker-compose.prod.yml up -d --build ops-brain`. Cleanup with `docker compose down` ALSO took out the recovered prod container (shared service name). Total downtime ~3 minutes.

  **What PR #33 fixed (2026-04-07):** Dev file is now project-namespaced as `ops-brain-dev` (`name:`, container names, network/volume all suffixed). A stray `docker compose up` from the wrong file on the prod host now spins up isolated `-dev` containers and leaves production untouched. Original incident no longer reproducible.

  **But the rule still stands.** Without `-f docker-compose.prod.yml` you don't get the prod stack — you get an isolated empty dev stack on a different network. Not destructive, but still wrong. Sanity-check after any deploy: `docker exec shared-postgres psql -U ops_brain -d ops_brain -c "SELECT count(*) FROM handoffs;"`. Zero → wrong DB.

  Cleaning up dev orphans on prod host: `docker stop ops-brain-dev ops-brain-dev-db; docker rm ...; docker network rm ops-brain-dev_default; docker volume rm ops-brain-dev_pgdata`. Never touches prod.

- **`docker-compose.prod.yml` enumerates env vars explicitly — no `env_file:`.** When wiring a new env var the binary reads at startup, the PR must touch both:

  1. `.env` on the prod host (actual value)
  2. `docker-compose.prod.yml` `services.ops-brain.environment:` (`- NEW_VAR=${NEW_VAR:-}` so compose substitutes from `.env` into the container)

  2026-05-04 rmcp 1.6 deploy (PR #47) hit this: the binary correctly read `OPS_BRAIN_ALLOWED_HOSTS`, `.env` had the value, but the container booted with `loopback default` because compose had no line to substitute. CC-Cloud caught it from startup log (`HTTP allowed_hosts: loopback default (set OPS_BRAIN_ALLOWED_HOSTS for public deploy)`) and shipped PR #48 (`e3eb232`). Fail-open guard did its job — degraded to loopback rather than allow-all.

  `/prereview` should flag a new `std::env::var(...)` call paired with no compose change. Spot-grep `docker-compose.prod.yml` before approving. The `${VAR:-}` form (empty default) is the right pattern — lets `.env` drive the value while keeping compose valid when unset.

## MCP Clients

- **`Session not found` after idle is server-side, not client-side.** rmcp 1.6's `SessionConfig::DEFAULT_KEEP_ALIVE` is 300s — `LocalSessionManager::default()` evicts every MCP session after 5 minutes of inactivity. Existing MCP clients (Claude Code's Rust rmcp HTTP client, Gemini CLI's Node `@modelcontextprotocol/sdk`, others) don't auto-reinitialize on the resulting 404; the user must reconnect (`/mcp` in CC). Earlier guess pinning this on Node `eventsource` timeouts or missing rmcp ping frames was wrong — Claude Code's Rust client hits the exact same eviction. Mitigated by the keep_alive bump in `src/main.rs` that sets `session_config.keep_alive = Some(Duration::from_secs(3600))` on the `LocalSessionManager`. If `Session not found` recurs anyway, check the new keep_alive is in the deployed binary, then look at rmcp release notes for further session-lifecycle changes.

- **MCP session evicted on container recreate — use the psql escape hatch for closeout.** When the deployer (CC-Cloud, Codex-Cloud) does `docker compose up -d --build ops-brain`, their existing MCP client session against the *running* container is wiped — the in-memory session table goes with the old process. Subsequent MCP tool calls 404 with `Session not found`. The CC MCP client does **not** reliably auto-reinitialize within the same turn. Plan the closeout to use the `shared-postgres` psql escape hatch for `complete_handoff` / `update_handoff` writes, not the MCP path.

  Three observations across deploys: v3.1.0 (`019e1e06` — session survived/reconnected cleanly, lucky), rmcp keep_alive bump (`5ab86c21` — needed escape hatch), v3.2.0 (`c254ceb7` — needed escape hatch). 2 of 3 recreates evicted the session; the v3.1.0 happy path is the outlier, not the norm. This is **unrelated** to the v3.2.0 HTTP `keep_alive` 5min → 1h bump — that fixes idle eviction, not eviction on container restart.

  Deploy skill should pre-stage the psql closeout (`docker exec shared-postgres psql -U ops_brain -d ops_brain -c "UPDATE handoffs SET status='completed', commit_hash='<sha>', updated_at=NOW() WHERE id='<uuid>'"`) so the deployer doesn't have to compose it under time pressure. After successful smoke, if `complete_handoff` MCP returns `Session not found`, drop straight to the escape hatch — do not retry MCP. Retry won't auto-reconnect inside the same turn.

  CC-Stealth's session (working remotely, separate container relationship) does NOT have this problem — only the deployer's session, which is co-located on the kensai.cloud host and has a live session-id against the just-killed process. Possible upstream fix: rmcp HTTP client could auto-reinitialize on 404, but that's out of scope for ops-brain itself.
