# Changelog

All notable changes to this project will be documented in this file.

## [Unreleased]

### Changed

- **MCP surface drift-and-bloat sweep.** The surfaces every cold-booting CC reads on connect — the server `instructions` block, the 64 `#[tool(description=...)]` strings in `src/tools/mod.rs`, the param `/// ` doc-comments that schemars serializes into JSON Schema, and the runtime error messages — had accumulated repetition, editorial prefixes, implementation jargon, CLAUDE.md-pointing hand-holding, and defensive v1.4-ritual warnings that the v1.5 architecture already makes impossible. Trimmed with the goal that a fresh CC sees only what actually helps them pick the right tool.
  - **Server `instructions`** — dropped the "small group of Claude Code instances running a real MSP" framing (CC team membership belongs in each CC's own CLAUDE.md), the "your CLAUDE.md is your scope / your filesystem is your state / your git history is your memory" parenthetical (same idea restated three ways), and the "if a question can be answered without ops-brain, it should be" closer (already said by "reach for ops-brain only when you need the rest of the team"). Block shrinks from ~420 to ~320 chars. Same substance: team bus identity, local-is-source-of-truth, when to reach, default-deny cross-client.
  - **`check_in` description** — biggest single cut. The v1.5 walkback had the description preaching "It is NOT a startup ritual and NOT required for any other tool — local is the source of truth, ops-brain is the team bus." at every cold boot. Redundant with the now-tighter server instructions, and the preaching tone itself was a drift artifact (nothing else in the tool surface calls anything first, so there's nothing to push back against anymore). Also dropped the "one of CC-Cloud, CC-Stealth, CC-HSR, CC-CPA — your CLAUDE.md tells you yours" enumeration in favor of plain "your CC name" — the team roster doesn't belong leaking into every tool listing.
  - **`get_situational_awareness` description** — dropped the `KEY TOOL:` editorial prefix (CCs pick based on what a tool does, not on self-promotion) and the `~94K→~10K` token-accounting leak (the phrase "reduce response size" is sufficient signal to use `compact`; the exact savings belong in REFERENCE.md, not in the tool description that ships on every list).
  - **`upsert_server` description** — dropped `(COALESCE — omitted fields are preserved)` in favor of plain "omitted fields are preserved" (SQL jargon leak), and the `"On create, NOT NULL fields default to empty/false/active"` clause (schema-internals leak; discoverable from a failed create + error message).
  - **`add_knowledge` description** — dropped `"(read it from your CLAUDE.md)"` from the `author_cc` guidance. The allowlist error already tells the caller the valid names if they get it wrong; the parenthetical was hand-holding that also contradicts the v1.5 principle that local CLAUDE.md is the source of truth (so pointing at it in every MCP description is noise).
  - **`delete_knowledge` description** — dropped `"Use with caution — this is permanent."` Every `delete_*` is permanent by definition. The warning was content-free ceremony.
  - **Response bodies audited** — grepped handler code for `"Next…"`, `"Consider…"`, `"Tip:"`, `"Remember to…"`, `"you should…"`, `"don't forget…"` and variants. Zero user-facing breadcrumbs found; the only hits were internal Rust `//` comments on provenance immutability. ops-brain already returns data, not nudges — good hygiene, nothing to cut on this surface.
  - **Param doc-comments (JSON-schema descriptions) sweep.** Param `/// ` doc comments are serialized by schemars into the JSON Schema shown to CCs at tool enumeration — they're a third surface every cold-boot CC reads, not just an internal dev-facing artifact. The same "read your CC name from your per-machine CLAUDE.md" hand-holding that got cut from the `add_knowledge` tool description was also duplicated on the `author_cc` param doc in `src/tools/knowledge.rs` **and** on the `my_name` param doc in `src/tools/cc_team.rs` **and** emitted at runtime as a trailing sentence on the `Invalid author_cc` error message. Third-location drift. Trimmed in all three places. The allowlist itself stays on the param docs (genuinely useful discovery at call time) and in the error message (tells the caller exactly what went wrong); what got cut was the CLAUDE.md pointer that repeats architectural doctrine on every listing/failure.
  - **Param doc consistency on cross-client `acknowledge_cross_client`.** Five of the six sites using this param had the same one-liner ("Release cross-client results withheld due to scope mismatch"); the sixth (`CreateIncidentParams` in `src/tools/incidents.rs`, added in PR #40 for the similar-incident surface) had an expanded four-line restatement of the gate mechanics the server instructions already describe. Aligned to the one-liner. No behavior change; just less text on the wire for every CC listing `create_incident`.
  - **No functional change.** No schema change, no tool added/removed/renamed, no parameter added/removed, no response-shape change. Cold `cargo check` passes clean. This is purely a reduction in the static text every CC reads on connect.

## [1.7.0] — 2026-04-08

**Similar-work surface on `create_incident`.** Prevents duplicate-effort races across the CC team: when a CC files a new incident, semantically similar OPEN incidents already in flight are surfaced in the response so the caller can link/dedupe via `link_incident` instead of working a parallel copy. Reuses the v1.5 cross-client gate, adds hit- and miss-side telemetry so the similarity threshold can be retuned from production data. Ships the first real use of the embedding infrastructure outside search.

### Added

- **Incident similarity surface on `create_incident`: `_similar_incidents`** (PR #40) — when a new incident is created, semantically similar OPEN incidents are surfaced in the response so the calling CC sees *"this looks like work already in flight"* and can choose to link/dedupe via `link_incident` instead of duplicating effort. Raid #3 of the post-PR#29 roadmap; closes the original 5-item list.
  - **New repo function `find_similar_open_incidents`** in `src/repo/embedding_repo.rs` — modeled on `find_similar_knowledge`. Returns up to 3 `SimilarIncident` rows (id, title, status, severity, client_id, cross_client_safe, created_at, distance) within cosine distance 0.30 (≈70% similarity) of the new incident's embedding. Threshold is **looser than knowledge's 0.15** because the goal here is "related work" not duplicate detection. Self-excludes the calling incident via `id != $2`, scopes to `status = 'open'`, skips NULL embeddings.
  - **Embedding is computed once and reused.** The handler previously called `embed_and_store` which re-hit the embedding API twice (once for storage, once for any similarity check). PR #40 replaces that with a single `embedding_client.embed_text()` call whose result is fed to *both* `find_similar_open_incidents` and `store_incident_embedding`. Same number of API calls as before, plus one HNSW lookup. Net cost: one vector query per `create_incident`.
  - **No schema change.** The `incidents.embedding` column, HNSW index, and embed-on-create flow already existed from earlier migrations. PR #40 rewires the handler pipeline without touching the schema. Migration count stays at 43.
  - **Cross-client gate reused from `search_knowledge` pattern.** The SQL returns matches across all clients; the gate is applied at the handler layer via `helpers::filter_cross_client`, with the new incident's `client_id` as the requesting scope. Withheld matches land in `_cross_client_withheld` with scope-mismatch notices and are audit-logged via `shared::log_audit_entries`. New handler param `acknowledge_cross_client: Option<bool>` releases withheld matches on an explicit second call — matches the two-call gate pattern used elsewhere.
  - **Response shape change.** `handle_create_incident` now returns `{ incident, _similar_incidents, _cross_client_withheld? }` instead of the bare incident JSON. `_similar_incidents` is **always present** (empty array when there are no matches, when the embedding client is absent, or when the embedding call fails) — guarded by an explicit `handler_create_incident_response_includes_similar_field` shape-contract test so callers can rely on the field being a stable part of the response contract.
  - **Best-effort on embedding-service outages.** If `embed_text` fails, a `warn!` is logged and the incident still creates successfully — `_similar_incidents` is just empty and the row's `embedding` column stays NULL. The watchdog's periodic backfill picks it up on the next pass.
  - **Hit-side telemetry.** `tracing::info!` logs `new_incident`, `similar_count`, and `min_distance` whenever matches surface, enabling the 0.30 threshold to be retuned from real production data. Miss-side telemetry follow-up tracked as its own entry below.
  - **Watchdog path untouched.** The watchdog creates incidents via `incident_repo::create_incident_with_source` directly, not through the `handle_create_incident` handler, so auto-generated incidents bypass the similarity surface by design. Only user-initiated `create_incident` tool calls get the new behavior.
  - **5 new integration tests** in `incident_similarity_tests` module: `returns_close_matches` (happy path), `self_excludes` (id != $exclude correctness), `excludes_resolved` (status='open' filter), `respects_distance_threshold` (orthogonal vectors stay out), `handler_create_incident_response_includes_similar_field` (response-shape contract guard for the no-embedding-client path). Tests use synthetic 768-dim embeddings with leading non-zero components for deterministic distance math — no real embedding service required. Suite: 39 → 44 passed.
  - **Visibility bump.** `tools::incidents` module moved from `mod` to `pub mod` in `src/tools/mod.rs`, and `handle_create_incident` from `pub(crate)` to `pub`, so integration tests can exercise the handler-level response-shape contract. Matches the existing `pub mod knowledge` / `pub mod cc_team` pattern.
- **Incident similarity: nearest-miss telemetry** (PR #41, commit `a709643`) — complements the hit-side `min_distance` log added with the `_similar_incidents` surface in PR #40. When `create_incident` runs the similarity query and finds nothing below the 0.30 cosine-distance cutoff, a second lightweight query (`nearest_open_incident_distance` in `src/repo/embedding_repo.rs`) returns the distance to the nearest open incident regardless of threshold, logged at `info` level with `new_incident`, `nearest_distance`, and `threshold` fields. Motivation: v1.7 smoke testing showed two intuitively related titles landing at 0.436, well above the 0.30 cutoff — hit-only telemetry can't distinguish "threshold is correctly tight" from "threshold is too tight, we're missing things." Miss-side telemetry gives the full distance distribution needed to retune 0.30 from data instead of guessing. Second query runs only on misses (empty match list), skipped entirely on cold start (`Ok(None)` is silent), and errors are non-fatal (`warn!` only — the incident creation still succeeds). No behavior change to the user-facing response shape. One new integration test in `incident_similarity_tests` covers the telemetry path.

## [1.6.0] — 2026-04-07

**Provenance on shared knowledge.** v1.5 established that local is the source of truth for everything *except* shared cross-CC knowledge; v1.6 closes the gap there. Every new knowledge entry is stamped with its author CC at create time, optionally linked to the incident that produced it, and surfaces a read-time staleness flag when it goes >90 days without a verify. Provenance on the one shared artifact that v1.5 had to leave shared.

### Added

- **Knowledge provenance: `author_cc`, `source_incident_id`, `_staleness_warning`** (migration `20260408000001`) — v1.5 established that local is the source of truth for everything *except* shared cross-CC knowledge; this release closes the provenance gap in that one shared artifact. Every new knowledge entry is stamped with its author CC at create time, optionally linked to the incident that produced it, and surfaces a read-time staleness flag when it goes >90 days without a verify.
  - **`author_cc` is required on `add_knowledge`** — validated against the `CC_TEAM` allowlist (`CC-Cloud`, `CC-Stealth`, `CC-HSR`, `CC-CPA`). Missing or unknown names fail loudly with the full allowlist in the error. Pre-v1.6 rows stay NULL — no forced backfill, no fabricated authorship. **Breaking change**: existing MCP callers that omit `author_cc` will see a clean error and need to pass their CC name (read from per-machine CLAUDE.md) on the first call after deploy. This is intentional: soft-defaulting to NULL would perpetuate the stealth bug we're closing.
  - **`author_cc` is immutable via the tool surface** — `UpdateKnowledgeParams` has no `author_cc` field, so the compiler itself guarantees the invariant. Provenance that can be rewritten isn't provenance. Emergency correction still possible via direct SQL.
  - **`source_incident_id` is optional and post-hoc updatable** — can be set at create time or added later via `update_knowledge`. FK with `ON DELETE SET NULL` on `incidents(id)` so cleaning up incidents doesn't cascade-delete the lessons learned from them. Handlers verify the incident exists before INSERT/UPDATE so failures are clean errors rather than raw FK violations.
  - **`_staleness_warning` is computed at read time** — `true` if `last_verified_at.unwrap_or(created_at)` is older than 90 days. No schema column, no background job, no drift. Surfaced in `search_knowledge` (single-table, multi-table, and browse modes), `list_knowledge`, and on every knowledge result returned through compact or non-compact paths. Threshold constant is `KNOWLEDGE_STALE_DAYS = 90` in `src/tools/knowledge.rs`.
  - **`CC_TEAM` allowlist helpers** exposed from `src/tools/cc_team.rs` as `pub(crate) fn is_valid_cc_name` and `pub(crate) fn cc_allowlist`, reusing the static `CC_TEAM` table instead of duplicating the 4-name list. The `knowledge` module uses them for validation without a DB-side membership table.
- **15 new tests** — 7 pure-logic unit tests in `src/tools/knowledge.rs` (staleness thresholds, off-by-one guards, JSON enrichment) + 8 handler-level integration tests in `tests/integration.rs::knowledge_provenance_tests` (allowlist validation, FK existence checks, author_cc immutability, post-hoc linking, staleness surfaced on list). Existing `knowledge_tests::add_and_get_knowledge` and `knowledge_cross_client_safe_field` updated for the new required repo param.

### Changed

- **`knowledge` module visibility: `mod knowledge` → `pub mod knowledge`** in `src/tools/mod.rs`, so handler-level integration tests can reach `handle_add_knowledge`, `handle_update_knowledge`, and `handle_list_knowledge`. Matches the existing `pub mod cc_team` pattern. Internal handlers moved from `pub(crate)` to `pub` for the same reason.
- **Migration count: 42 → 43** (the knowledge-provenance migration `20260408000001`). Tool count unchanged at 64 (new params on existing tools, no new tools). No fields added to `check_in` (hard-stop respected).

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
