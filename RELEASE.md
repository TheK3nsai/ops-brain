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
- [x] **Rewrite `seed/seed.sql`** — replaced real client names with fictional examples (Acme Healthcare, Summit CPA), RFC 1918 IPs, generic site names
- [x] **Scrub `CLAUDE.md`** — rewrote as public contributor guide. Removed: Zammad IDs, trigger IDs, email, real machine names, real URLs, deployment details, phase history, infrastructure audits. Private ops data kept locally.
- [x] **Scrub `README.md`** — replaced `kensai.cloud` URLs with `example.com`, replaced real slugs/client names with generic, cleaned roadmap references
- [x] **Scrub source code** (all locations cleaned):
  - `src/tools/mod.rs` — MCP server instructions routing genericized, test client names → Alpha Corp/Beta Inc
  - `src/tools/zammad.rs` — hardcoded `owner_id: Some(3)` → configurable via `ZAMMAD_DEFAULT_OWNER_ID` env var
  - `src/tools/zammad.rs:40` — tags are generic IT categories, kept as-is
  - `src/tools/inventory.rs:14` — slug examples → "web-server-01", "db-primary"
  - `src/tools/coordination.rs:14` — machine examples → "dev-laptop", "prod-server"
  - `src/config.rs:26,38` — URLs → example.com
  - `src/metrics.rs:236-248` — test URLs → example.com
  - `src/zammad.rs:551` — test group "HSR" → "Support"
  - `src/embeddings.rs` — test machine names → "dev-laptop", "prod-server"
- [x] **Scrub integration tests** (`tests/integration.rs`) — replaced stealth/cloudlab/HSR/CC-Stealth/CC-HSR with generic names
- [x] **Scrub migrations** (2 files):
  - `20260327000001` — removed `%hvdc01%` pattern (deduplicated with `%dc outage%`)
  - `20260327300001` — scope comment examples genericized
  - **Decision**: keep migrations as-is (only 2 trivial references, squash happens via git history clean)

### Git History
- [ ] **Clean squash to fresh repo** — old seed.sql in git history contains real infrastructure data (IPs, server names, runbook procedures, incident details). No secrets, but operational security concern. Squash eliminates this.

## Pre-Release: Code Quality

- [x] **`cargo clippy --all-targets`** — zero warnings (verified 2026-03-28)
- [x] **`cargo audit`** — clean (1 ignored, non-exploitable)
- [x] **Tool descriptions compacted** — ~3,500 chars saved across 30+ tools and params (2026-03-28)
- [x] **SECURITY.md** — responsible disclosure process, security contact
- [x] **LICENSE** — dual MIT/Apache-2.0 (LICENSE-MIT + LICENSE-APACHE), Cargo.toml updated
- [x] **Docker-compose for new users** — `docker-compose.yml` with PostgreSQL+pgvector+ops-brain, all integrations optional via env
- [ ] **Verify fresh `docker compose up` works** with only `.env.example` values
- [x] **Version bump** — Cargo.toml 0.1.0 → 1.0.0

## Pre-Release: Documentation

- [ ] **README.md rewrite for public audience** — current README is comprehensive but could use: quick start section, screenshots/demo, feature overview aimed at new users
- [ ] **CONTRIBUTING.md** — optional, CLAUDE.md already has a full Contributing section
- [ ] **Update repo visibility** — flip from Private to Public on GitHub

## Remaining Before Public Release

1. **Git history clean squash** — the last blocker. Creates a fresh repo with clean history.
2. **Verify `docker compose up`** — quick smoke test with `.env.example` values only.
3. **Rotate Zammad API token** — post-release precaution.

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
- [ ] **Multi-instance Uptime Kuma** — connect multiple client Kuma instances
- [ ] **Web dashboard** — view ops-brain data without a Claude session
- [ ] **Auto-deploy on merge** — GitHub Actions CD
- [ ] **Claude.ai read-only integration** — read-only MCP or REST API for strategic analysis layer
