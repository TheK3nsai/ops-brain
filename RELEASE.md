# ops-brain v1.0 Release Checklist

Target: Public open-source release on GitHub.

## Pre-Release: Security & Privacy (MUST DO)

### Secrets Audit
- [x] **No secrets in git history** — verified 2026-03-28, full `git log --all -p` scan
- [x] **`.env` properly gitignored** — never committed
- [x] **`.env.example` clean** — placeholder values only
- [x] **CI passwords are ephemeral** — `POSTGRES_PASSWORD: ops_brain` in CI only
- [ ] **Rotate Zammad API token** after repo goes public (precaution)
- [x] **cargo audit** — 1 medium vuln (rsa/Marvin Attack via sqlx-mysql transitive dep, not exploitable, ignored in CI)

### Client Data Scrub
- [ ] **Rewrite `seed/seed.sql`** — replace real client names (HSR, CPA), site names, cities, IPs, Zammad IDs with fictional examples (e.g. "Acme Healthcare", "Demo CPA", RFC 5737 IPs like 192.0.2.0/24)
- [ ] **Scrub `CLAUDE.md`** — split into public contributor guide + private ops doc (kept outside repo). Remove: Zammad IDs, trigger IDs, email, real machine names, real URLs, real server names, IP addresses
- [ ] **Scrub `README.md`** (~6 refs) — replace `kensai.cloud` URLs with `ops.example.com`, replace `hvfs0`/`hsr` examples with generic slugs, update GitHub clone URL
- [ ] **Scrub source code** (~8 locations across 6 .rs files):
  - `src/tools/mod.rs:938-939` — MCP server instructions routing table (real machine names)
  - `src/tools/zammad.rs:254` — hardcoded `owner_id: Some(3)` → make configurable via env
  - `src/tools/zammad.rs:40` — real Zammad tag examples in doc comment
  - `src/tools/inventory.rs:14` — real server slug examples in doc comment
  - `src/tools/runbooks.rs:90` — real client slug examples in doc comment
  - `src/config.rs:26,38` — real URLs in doc comment examples
  - `src/metrics.rs:236-248` — real URLs/ports in test fixtures
  - `src/zammad.rs:551` — real Zammad group in test fixture
- [ ] **Scrub integration tests** (`tests/integration.rs`) — replace HSR references in test data
- [ ] **Scrub migrations** (2 files with real hostnames):
  - `migrations/20260327000001_add_source_recurrence_to_incidents.sql:21,30` — `%hvdc01%` pattern
  - `migrations/20260327300001_staleness_preferences_backlog.sql:11` — real machine/client in comment
  - **Decision needed**: squash all 35 migrations into one clean schema, or leave as-is (only affects pre-existing data)

### Git History
- [ ] **Clean squash to fresh repo** — old seed.sql in git history contains real infrastructure data (IPs, server names, runbook procedures, incident details). No secrets, but operational security concern. Squash eliminates this.

## Pre-Release: Code Quality

- [x] **`cargo clippy --all-targets`** — zero warnings (verified 2026-03-28)
- [x] **`cargo audit`** — clean (1 ignored, non-exploitable)
- [x] **Tool descriptions compacted** — ~3,500 chars saved across 30+ tools and params (2026-03-28)
- [ ] **SECURITY.md** — responsible disclosure process, security contact
- [ ] **LICENSE** — choose and add (MIT? Apache-2.0? dual?)
- [ ] **Docker-compose for new users** — `docker-compose.yml` that works out of the box with PostgreSQL + pgvector + ops-brain
- [ ] **Verify fresh `docker compose up` works** with only `.env.example` values

## Pre-Release: Documentation

- [ ] **README.md rewrite for public audience** — remove internal ops details, add: quick start, screenshots/demo, feature overview, comparison to alternatives
- [ ] **CONTRIBUTING.md** — extract from CLAUDE.md's "Contributing" section, add human contributor guidance
- [ ] **Update repo visibility** — `Private. Open-source release planned when polished.` → remove this line

## Post-v1.0 Backlog

These do NOT block release. Ship first, iterate after.

### Testing (P1 post-release)
- [ ] **Client-scope gate integration tests** — all 5 gate conditions (global, same-client, cross-safe, cross-ack, cross-withheld)
- [ ] **Watchdog logic tests** — flap suppression, severity degradation, dedup/reopen
- [ ] **Soft delete tests** — FK preservation, audit trail
- [ ] **Search tests** — hybrid RRF ranking sanity, OR fallback triggers

### Architecture & Docs
- [ ] **ARCHITECTURE.md** — domain model, data flow, why decisions were made (especially client-scope gate rationale)
- [ ] **Self-ops runbook** — backup, restore, migration rollback, watchdog emergency procedures

### Infrastructure
- [ ] **Backup/DR for ops-brain database** — automated PostgreSQL backups with tested restores
- [ ] **Dynamic tool loading** — profile-based tool registration to reduce context window overhead (rmcp architectural change)
- [ ] **Multi-instance Uptime Kuma** — connect to HSR's separate instance at status.ihmpr.com
- [ ] **Web dashboard** — view ops-brain data without a Claude session
- [ ] **Auto-deploy on merge** — GitHub Actions CD to kensai.cloud
- [ ] **Claude.ai read-only integration** — read-only MCP or REST API for strategic analysis layer
