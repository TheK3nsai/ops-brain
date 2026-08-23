# TODO

Open work only. Shipped history lives in `CHANGELOG.md`, doctrine and hard stops in `ROADMAP.md`, code and deploy footguns in `GOTCHAS.md`.

**Release-intent doctrine, still in force:** _"don't sharpen for optics; build for the 4 CCs."_ The external-user release sweep shipped 2026-05-18 (#56, #57). No further external-polish work without actual external signal — someone files an issue, asks for help, or otherwise shows up.

## Open

- **Pre-scrub commit object may still be reachable on GitHub (2026-08-23, PR #85).** The fleet-private string guard caught a client hostname in a `GOTCHAS.md` entry *after* the branch was first pushed. The working tree, commit message, and PR body were scrubbed and force-pushed (guard now passes), but the original object stays reachable by exact SHA on a public repo until GitHub garbage-collects it. Low severity — one hostname, no credential — and the only real remedy is a GitHub Support GC request, so this is Eduardo's call whether to bother. Durable lesson worth more than the cleanup: **the guard runs in CI, i.e. after the push that publishes the string.** The pre-commit hook enforces the same denylist locally for ops-brain, but it only scans the working tree — it does not see the commit message or PR body, which is exactly where this one also landed. If this recurs, extend the local hook to the commit message rather than relying on the CI guard.

## Don't re-propose without new evidence

Deliberate decisions with their reasons. If real friction ever shows up, re-open the question from first principles — don't resurrect the design.

**Self-hosting the alert relay on the monitored box (2026-07-29).** When
picking a notification provider for Kuma, self-hosted ntfy behind the existing
Caddy looks like the tidy, no-third-party answer. It isn't: **it shares fate
with Caddy, and Caddy is one of the monitored services.** Caddy down, cert
expired, proxy misconfigured — the alert about it cannot get out, because it
routes through the thing that broke. That is the #77 defect wearing a different
hat, and the generalization is worth more than the specific call: *an alert
path must not traverse anything it is responsible for reporting on.*

Hosted ntfy.sh needs only outbound HTTPS from the container, so it survives
every partial failure short of host death — and host death takes Kuma with it
regardless of provider, which is why that gap is scoped out separately. It also
keeps the off-box-check door open: an external checker can publish to the same
topic, which a relay living on the monitored box forecloses.

The privacy tradeoff is bounded by *what we send*, not by a promise about ntfy:
monitor names and short status strings about our own services, no client data,
no PHI. If a monitor ever carries client-identifying content, re-run this
reasoning rather than inheriting it.

Pushover was the closest alternative — stable tokens, real auth rather than an
unguessable topic — and is the upgrade path if obscurity-based auth ever
bothers us. Declined because it reintroduces a credential to store and rotate,
and "a credential that died quietly" is the failure being fixed.

**Four-lens audit declines (2026-07-17, v4.1.0, PRs #64/#65/#67).** Every finding was fixed, tested, or consciously declined. The declines:

- **Verbatim DB error strings to callers** — every caller is one of our own authenticated agents; informative errors beat sanitization theater here.
- **`build_client_lookup` caching / backfill batching / embedding retry-backoff** — real, but irrelevant at 4-agent scale. The perf audit put these on its own "theater" list.
- **Unified error enum** — the current boundary-appropriate typing (`sqlx::Error` → `CallToolResult` / `(StatusCode, String)`) is deliberate, not mess.
- **Briefing cosmetic nits** — a section is omitted when a LIMIT-20 page holds none of a counted status; the unscoped `_note` yields to the rarer embedding note on multi-table. Edge-case cosmetic, flagged in #67's body.

**Automation backbone, phase 3 (2026-07-17, joint design with CC-HSR).** Phases 1+2 shipped; producer contract in `docs/machine-callers.md`. Deliberately not built: server-side recurrence (producers' own schedulers plus `dedupe_key` idempotency cover it), structured handoff columns (the versioned `context` convention instead — promote fields only if a pilot bleeds from prose-parsing), and webhooks-out (poll `GET /api/pending`; an additive upgrade if sub-minute latency ever becomes real pain).

**Operator visibility, tier 2 (2026-07-28).** A log or view of everything happening headless. The handoffs table already *is* the log — `origin`, `status`, `repeat_count`, threading, timestamps. The only real gap is that `updated_at` is destructive, so there's no `accepted_at` and "how long did this sit on a human" isn't answerable. That's a schema change in service of a metric nothing is bleeding from. If friction shows up, the cheapest next step is a section in the briefing that already gets read — not a web view, not event sourcing. Full reasoning in `docs/operator-notify.md`.

## Closed

- **Kuma's alerts reach no human** — reopened 2026-08-02, **closed 2026-08-03**. The 08-02 reopen was correct: the provider delivered to the ntfy topic (verified by polling the message back off ntfy.sh) while no device had ever subscribed, and ntfy's 200-for-any-topic makes those two states identical from the box. The operator subscribed on 08-03; the cloud host's `TODO.md` records the follow-on decisions taken "with alerting finally reaching a phone", and `kuma-watchdog.sh` was built on top of it. Closed here on that operator confirmation — **there is no server-side test that could close it**, which is the durable lesson (gotcha `019fc421-a935`, generalizing to any alert path terminating off-box). Fixed en route: `resend_interval = 0` on all 32 monitors, the Kuma default that alerts once on the DOWN transition and then goes silent forever — now ~6h via `max(1, round(21600 / interval))`, counting consecutive failed checks rather than minutes; mechanics in the cloud host's `docker/CLAUDE.md`. **Residual, tracked and deliberately open elsewhere:** nothing verifies the phone *stays* subscribed, so iOS delivery rot would silently return us to "alerts nowhere" with every server signal green. That is logged as an accepted gap in the cloud host's `TODO.md` (2026-08-03) — don't re-file it here as newly discovered.

- **The operator notifier shares fate with the channel it reports on** — 2026-07-29, #79, live the same day. Opened 07-28 during the #77 deploy: one revoked Gmail app password killed the briefing, the security digest, and operator-notify at once, so the handoff reporting "outbound ops email is dead" could not be delivered *because of the condition it was reporting*. Nothing was ever lost — a failed send holds the cursor and retries — but a dead notifier and a quiet bus looked identical. Resolved **without** the second credential the item was weighing: the escalation channel already existed and was already exercised daily (Uptime Kuma, which the sibling wake shim has pushed to since day one, and which owns monitoring by doctrine). `operator-notify.sh` gained an optional `$OPS_NOTIFY_HEARTBEAT_URL` it pings `up`/`down` on every real run — no Rust, no MCP surface, no failure counter, no outage-only path that could rot unnoticed. Thresholds and routing stay in the product that owns them. Control-tested 12/12 including unchanged cursor retry semantics and that an unreachable monitor is not a new failure mode. Kuma's side landed the same day: hosted ntfy.sh as a default-enabled provider, `Operator Notify` push monitor (1800s/1), crontab sourcing `conf/.env` so the push token stays out of a `ps`-visible command line. Setup gotchas — including the "Default enabled" backfill trap — in `docs/operator-notify.md`. **Correction (2026-08-02): "Kuma's side landed" overclaimed.** The provider was wired correctly and does deliver to the topic — verified — but no device was ever subscribed, so nothing reached a human. The ops-brain half of this item (`$OPS_NOTIFY_HEARTBEAT_URL`) is genuinely closed; the Kuma half was reopened 08-02 and closed 08-03 — see the entry above it. **Scope held:** this covers a dead *channel*, not a dead *host*; if the box dies, monitor and notifier die together, which wants an off-box check and is a different problem.

- **Operator visibility, tier 1** — 2026-07-28, #77. The design session resolved it to a convention plus a cron with **zero lines of Rust**: agents reply into the existing thread addressed to the operator's slug, and a read-scoped machine token plus a cron polls that queue and mails a digest. Contract in `docs/operator-notify.md`, poller in `scripts/operator-notify.sh`. Deployment and token mint handed to CC-Cloud (`019fa9fe`).
- **Per-agent MCP tokens with server-bound `from_agent`** — 2026-07-28, #73 and #75, released in v4.2.0. Deployed 2026-07-21; minting completed fleet-wide 2026-07-24 (prod boots `agent tokens configured count=8`), binding control-tested per host, and revoked tokens verified dead by probing for 401 rather than assumed. Remaining residuals are ops on other lanes: CC-HSR's local install of the re-minted pair (`019f95df`) and demoting the old shared bearer to break-glass.
- **Case-insensitive agent matching on handoff queries** — 2026-07-17, #64. `bb9a25b` had already made the agent filters `ILIKE`; the residual `_`-wildcard over-match on the MCP list filters is closed with `LOWER() = LOWER()` exact matching, plus an integration test pinning that `agent_x` no longer matches `agentXx`.
- **External-user release sweep** — 2026-05-18, #56 and #57. Repo description and topics, GitHub Releases backfill, README lede and quick-start, GHCR multi-arch pipeline, standalone `docker-compose.example.yml`. The `v3.2.0` image is live and public at `ghcr.io/thek3nsai/ops-brain`.
- **Node 20 → Node 24 action versions** — bumped to explicit Node-24 majors across `.github/workflows/` ahead of GitHub's 2026-06-02 auto-force.
