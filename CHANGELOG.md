# Changelog

All notable changes to this project will be documented in this file.

## [1.2.1] — 2026-03-30

### Added

- **`delete_handoff` tool** — hard delete handoffs by ID, requested after duplicate handoff cleanup pain

### Changed

- Tool count: 73 → 74

## [1.2.0] — 2026-03-29

### Added

- **Multi-instance Uptime Kuma** — watchdog and monitoring tools aggregate from multiple Kuma instances via `UPTIME_KUMA_INSTANCES` JSON env var. Single `UPTIME_KUMA_URL` still works for backward compat. Partial failure tolerant.
- **`upsert_network` tool** — create or update networks by slug (PR #22)
- **`source_url` on runbooks** — runbooks can reference canonical external docs
- **Staleness tracking** for knowledge & services — `last_verified_at` column with tiered briefing alerts (runbooks 30d, knowledge 60d, services 90d). `update_knowledge(verified=true)` and `upsert_service(verified=true)` mark entries as verified.
- Sessions & coordination section in CLAUDE.md

### Changed

- Tool count: 72 → 73 (`upsert_network`)
- Migration count: 35 → 39
- `docker-compose.prod.yml` uses `UPTIME_KUMA_INSTANCES` (replaces single-instance env vars)

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
