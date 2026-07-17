# TODO

External-user release sweep (#56 + #57) shipped on 2026-05-18: repo description, topics, GitHub Releases backfilled, README lede + quick-start, GHCR multi-arch image pipeline, standalone `docker-compose.example.yml`. The release-intent doctrine — _"don't sharpen for optics; build for the 4 CCs"_ — still applies: no further external-polish work without actual external signal (someone files an issue, asks for help, or otherwise shows up).

## Open

- **Automation-backbone charter (2026-07-17) — phases 1+2 shipped on `feat/machine-filed-handoffs`; phase 3 evidence-gated.** Joint design with CC-HSR (handoff thread `019f7094` → `019f709f`): machine-filed handoffs + wake poll, contract in `docs/machine-callers.md`. Deliberately NOT built (doctrine intact): server-side recurrence (producers' own schedulers + `dedupe_key` idempotency cover it), structured handoff columns (versioned `context` convention instead — promote fields only if the pilot bleeds from prose-parsing), webhooks-out (poll `GET /api/pending`; additive upgrade if sub-minute latency ever becomes real pain). Pilot consumer: CC-HSR's nightly backup-posture sweep; acceptance = file FAIL → wake poll shows it → repeat suppresses+bumps → complete → next FAIL files fresh. **Stealth wake path smoke-verified 2026-07-17**: Stealth-WS machine token live on stealth, `019f70e2` smoke handoff filed via `POST /api/handoff` → 5-min poll → headless CC-Stealth wake → accept/reply/complete, full loop green.

- **Case-insensitive agent matching on handoff queries.** The fleet convention is `Codex-HSR`-style, but prod data already has mixed casings (`Codex-HSR` and `codex-hsr` both live as distinct `from_agent` values), and `list_handoffs` `from_agent`/`to_agent` filters are exact-match — a targeted query silently misses the other casing. Bit CC-Stealth 2026-07-13 while hunting a CC-HSR handoff. Fix: `ILIKE`/`LOWER()=LOWER()` on the agent filters (cheap, no schema change); optionally lowercase-normalize `check_in`/`list_replies_to_me` the same way. Until shipped: query both casings or fall back to `search_handoffs`.

## Closed (2026-05-18 release sweep)

- ~~Tier 1: repo description + topics, GitHub Releases backfill for v1.7.0/v1.8.0/v3.2.0, README lede + quick-start.~~ Shipped in #56.
- ~~Tier 2: GHCR multi-arch release pipeline, standalone `docker-compose.example.yml`, README quick-start refresh.~~ Shipped in #57; `v3.2.0` image live and public at `ghcr.io/thek3nsai/ops-brain`.
- ~~Node 20 → Node 24 action versions across `.github/workflows/`.~~ Bumped to explicit Node-24 majors ahead of GitHub's 2026-06-02 auto-force: checkout v5→v6, upload-artifact v4→v7, download-artifact v4→v8, setup-buildx-action v3→v4, login-action v3→v4, build-push-action v6→v7, action-gh-release v2→v3. `actions/cache@v5` already on Node 24.
