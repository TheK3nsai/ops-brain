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

## MCP Clients

- **Gemini CLI (Node.js SDK) SSE Disconnects:** Gemini CLI instances (which rely on the `@modelcontextprotocol/sdk` Node.js client) might experience intermittent, silent SSE disconnects from `ops-brain` if left idle. This manifests as a 401 or `Session not found` error on the *next* tool call you make. This is likely an interaction between the Node `eventsource` package timeout and a lack of periodic ping frames from the `rmcp` server. If this happens, restart your CLI session to establish a fresh MCP connection.
