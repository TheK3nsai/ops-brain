# Changelog

All notable changes to this project will be documented in this file.

## [1.1.0] — 2026-03-28

### Security

- `search_inventory` now applies cross-client gating on runbooks, knowledge, and incidents

### Fixed

- `upsert_vendor` no longer creates duplicate rows (ON CONFLICT on LOWER(name))
- `upsert_server` preserves existing fields on update (COALESCE partial update)
- Connection pool bumped from 10 to 20 (prevents saturation during concurrent sessions)
- Test isolation documentation corrected (was claiming transaction rollback that didn't exist)

## [1.0.0] — 2026-03-28

Initial public release.

### Features

- **72 MCP tools** across inventory, runbooks, knowledge, incidents, monitoring, ticketing, briefings, and coordination
- **Hybrid search** — full-text (tsvector + websearch_to_tsquery) combined with semantic search (pgvector + nomic-embed-text) via Reciprocal Rank Fusion
- **Proactive monitoring** — background watchdog polls Uptime Kuma, auto-creates/resolves incidents with flap suppression (grace period, cooldown, deduplication) and severity logic based on server roles
- **Cross-machine coordination** — sessions and handoffs for multi-instance Claude Code collaboration
- **Client-scope safety gate** — default-deny cross-client data surfacing with explicit acknowledgment, audit logging, and provenance fields on all results
- **Zammad ticketing integration** — ticket CRUD, search, and bi-directional linking to incidents/servers/services
- **Scheduled briefings** — daily/weekly operational summaries via MCP tool or REST API (`POST /api/briefing`)
- **Fuzzy slug matching** — typo-tolerant lookups with "Did you mean: ...?" suggestions via pg_trgm
- **Docker Compose quickstart** — single `docker compose up -d` for PostgreSQL + pgvector + ops-brain
- **Dual transport** — stdio (local MCP) and streamable HTTP (remote/Docker)
- **35 database migrations** — auto-run on startup, idempotent
- **CI pipeline** — GitHub Actions with fmt, clippy, test, and cargo-audit
