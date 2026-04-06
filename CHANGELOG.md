# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Added

- **`check_in` tool** — every CC's first action of every session. Self-declares CC name, returns a one-shot briefing: your self-authored scope, the team roster, your open handoffs, your open incidents in scope. The whole morning ritual collapses into one call.
- **`set_my_identity` tool** — each CC writes its own confident scope (20-2000 chars, markdown). Self-sovereign by construction: a CC can only ever update its own row. Peers see this on every check_in.
- **`cc_identities` table** (migration `20260406000001`) — TEXT PK on cc_name, holds the four self-authored team identities.
- **First-write announcement handoffs** — when a CC writes its identity for the first time, ops-brain fans out a low-priority intro handoff to every other CC's machine. Best-effort, per-peer failures logged but don't fail the parent call.
- **Per-session identity state** on `OpsBrain` — `Arc<RwLock<Option<String>>>` populated by `check_in`, consumed by `set_my_identity`. Per-session because `StreamableHttpService` constructs a fresh `OpsBrain` per connection.

### Changed

- **`get_info` instructions slimmed from ~170 words to ~50** — the only thing the static string says now is "you're on a team, call check_in first." Tool list, knowledge policy, coordination protocol, compliance gate all moved to where they belong (knowledge entries, the briefing payload, or the code itself). The instructions are now an invitation, not a manual.
- Tool count: 65 → 67

## [1.3.0] — 2026-04-05

### Removed

- **Session tools** (`start_session`, `end_session`, `list_sessions`) — sessions were optional ceremony; handoffs are the real coordination layer
- **Runbook execution tools** (`log_runbook_execution`, `list_runbook_executions`) — audit trail nobody reviewed
- **Briefing browse tools** (`list_briefings`, `get_briefing`) — briefings delivered via server-side cron email; on-demand `generate_briefing` retained
- **Zammad pass-through tools** (`update_ticket`, `add_ticket_note`) — use Zammad UI directly for updates/notes; create, search, and linking tools retained
- Dead repo modules, model files, and integration tests for removed tools

### Changed

- Tool count: 74 → 65 (reduced token footprint for all CC instances)
- Updated server instructions to remove session references
- `get_catchup` stale runbook tip updated to reference `update_runbook(verified=true)` instead of removed `log_runbook_execution`

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
