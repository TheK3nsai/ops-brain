# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

## [1.5.0] — 2026-04-07

**ops-brain is the team bus, not a brain.** Local is the source of truth for each CC — its CLAUDE.md is its scope, its filesystem is its state, its git history is its memory. ops-brain exists for things CCs genuinely cannot do alone: handoffs to each other, shared incidents, cross-client knowledge with isolation rules, monitors, tickets that span systems. If a question can be answered without ops-brain, it should be.

This release walks back the v1.4 cc-team-identity work (`set_my_identity`, `cc_identities`, the "morning ritual" framing) which trained CCs to treat ops-brain as the source of truth for their own identity and scope. That was backwards. Local is authoritative; ops-brain is the bus. The surviving good ideas from the v1.4 era — the action/notify handoff split, the runbook advisory, the dev compose hardening — are kept as-is.

### Removed

- **`set_my_identity` tool** — identity and self-authored scope live in each CC's per-machine CLAUDE.md, not in a database row. The previous design created drift between the source of truth (CLAUDE.md) and a stale shared copy.
- **`cc_identities` table** (new migration `20260407000001` drops it) — held the data backing `set_my_identity`. Zero retention value: every row was free-text scope description that's also (and authoritatively) in the corresponding CLAUDE.md.
- **First-write announcement handoffs** — ops-brain no longer fans out intro handoffs when a CC "writes its identity," because there's no identity to write.
- **Per-session identity state** (`OpsBrain.cc_name: Arc<RwLock<Option<String>>>`) — there are no remaining readers; `check_in` now takes `my_name` as a stateless query parameter and other tools never needed it.
- **`set_preference` tool** + **`preferences` table** (dropped in same migration as `cc_identities`) + **`preference_repo`** + **`resolve_compact` helper** — preferences like `compact=true` were stored server-side and silently mutated other CCs' tool defaults. They belong in each CC's CLAUDE.md or per-call parameters, not in a centrally-shared row. Each `compact` callsite now resolves explicit param → hardcoded default with `unwrap_or`.
- **`get_catchup` tool** — overlapped with `check_in` (handoffs/incidents) and `search_*` for the rest. Proactive CCs don't need a "what changed since X" report; they do the work and check pending state via `check_in` if needed.

### Changed

- **`check_in` is now a stateless pending-work query.** Returns three things, scoped to your machine: open action handoffs, recent notify-class handoffs (compact: id/title/from/created_at), open incidents in your scope. No identity payload, no team roster, no self-authored scope. Tool description explicitly says it is **NOT a startup ritual and NOT required for any other tool** — local is the source of truth, ops-brain is the team bus. The `my_name` parameter remains so the query can scope to the right machine and client.
- **Server-level MCP `get_info()` instructions rewritten** to state the team-bus mental model in three sentences: what ops-brain is (and that local is the source of truth), when to reach for it, and the cross-client default-deny gate. Replaces the v1.4 "first action of every session" / "morning ritual" framing.
- **Repo `CLAUDE.md` Coordination section** rewritten with the team-bus framing and an explicit "no startup ritual" rule. The old "Startup is adaptive — check handoffs at a natural pause" wording remains compatible, but the new framing is clearer about *why*.
- **`ops-dev` agent guide** — "Database tables" guidance updated. Keeps "never modify existing migrations" (hard rule, checksum mismatch breaks deploys), but allows dropping tables via NEW migration when the data has zero retention value. Rule of thumb: when in doubt, leave the table.
- Tool count: 67 → 64 (3 deleted: `set_my_identity`, `set_preference`, `get_catchup`; `check_in` gutted to a stateless query). Migration count: 41 → 42.

### Added

- **`warnings` field on `create_runbook` / `update_runbook` responses** (PR #34) — when the runbook body exceeds 2 KiB and `source_url` is null/blank, the response includes a `warnings` array with an advisory pointing the caller at the canonicalization rule (runbook bodies of that size should live in a git-tracked file, with `source_url` set to the canonical path; the ops-brain entry should be a summary + pointer). Threshold is `RUNBOOK_INLINE_BODY_WARN_BYTES = 2048`. Whitespace-only `source_url` is treated as missing. Wired into both `create_runbook` and `update_runbook` (the latter evaluates the *merged* state of the persisted runbook, since the caller may have updated only one of `content` / `source_url`). Advisory only — the runbook is still created/updated as requested. Response shape is additive, backwards-compatible. Originated from CC-HSR's hygiene handoff after the first HSR offboarding runbook landed as a 340-line inlined body with no source_url.
- **Handoff `category` column — `action` vs `notify` split** (migration `20260406000002`) — `action` (default) is the existing persistent semantics; `notify` is for ephemeral FYIs (introductions, watchdog drops, "I just shipped X" announcements). `list_handoffs` defaults to action-only; pass `include_notify=true` or `category="notify"` to see them. Notify-class older than 7 days is filtered out at read time (rows preserved for audit/search). `check_in` surfaces action handoffs in `open_handoffs_to_you` and a compact `recent_notifications` block. Composite index `idx_handoffs_category_status` for the hot path.
- **`docker-compose.yml` (dev / new-user file) hardened against the prod-clobber footgun** (PR #33) — top-level `name: ops-brain-dev` namespaces the project so the default network becomes `ops-brain-dev_default` and the volume becomes `ops-brain-dev_pgdata`. `container_name` for the two services renamed to `ops-brain-dev` / `ops-brain-dev-db`. Header comment expanded with USE WHEN / DO NOT USE WHEN sections and an explicit pointer to `docker-compose.prod.yml`. Result: a stray `docker compose up` from the dev file on the prod host now spins up isolated `-dev` containers instead of recreating the production `ops-brain` container against an empty bundled postgres. Public surface unchanged. `docker-compose.prod.yml` is untouched.

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
