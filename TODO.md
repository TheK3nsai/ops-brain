# TODO

Open work only, a few lines each. Shipped history lives in `CHANGELOG.md`; doctrine, hard stops and decided-don't-re-propose items in `ROADMAP.md`; code and deploy footguns in `GOTCHAS.md`. Long working notes for an open item go in `docs/investigations/` and are deleted with the item.

**Release intent:** the existing fleet. Distribution is frozen at what updates itself (`ROADMAP.md` → *Decided*); do not add public-service or multi-tenant infrastructure.

## Open

- **DB index hygiene (bundle with next migration, if any):** `idx_handoffs_to_agent` is case-sensitive and unused by `LOWER(to_agent)` queries; `idx_handoffs_embedding` (HNSW) is dead weight since v8.0.0. When a schema migration is next needed, drop both and add partial open-queue `(LOWER(to_agent), created_at DESC) WHERE status IN ('pending', 'accepted')` and timeline `(created_at DESC, id DESC)` indexes. Not urgent at current table size.
- **`rmcp` 3.9 → 3.12+ upgrade:** compatible update is available; check release notes, confirm `cargo tree -i http` stays on one version, and re-verify live client reconnect before bumping.

(`operator-notify.sh` stays once-only by design — the daily briefing and bus reader carry the standing "waiting on you" view; see `docs/operator-notify.md`.)
