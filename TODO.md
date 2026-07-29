# TODO

Open work only. Shipped history lives in `CHANGELOG.md`, doctrine and hard stops in `ROADMAP.md`, code and deploy footguns in `GOTCHAS.md`.

**Release-intent doctrine, still in force:** _"don't sharpen for optics; build for the 4 CCs."_ The external-user release sweep shipped 2026-05-18 (#56, #57). No further external-polish work without actual external signal — someone files an issue, asks for help, or otherwise shows up.

## Open

Nothing open. The operator-notification thread closed 2026-07-29 — see below.

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

- **Uptime Kuma had no notification provider** — 2026-07-29. The load-bearing
  half of the item below, and the one that made it real rather than latent.
  Kuma ran **30 monitors** (12 push dead-man, including `Wake Shim`) with an
  **empty `notification` table**: it alerted nowhere and was a dashboard
  someone had to remember to open. `System Updates`, `Container Resources`,
  and `Disk Usage` had been red for hours with nobody told — the same silence
  as a dead mail credential, a different cause.

  Closed with hosted ntfy.sh as a default-enabled provider, 128-bit topic in
  `~/ops/conf/.env` (0600). Plus the `Operator Notify` push monitor (id 39,
  1800s / 1 retry) and the crontab rewritten to source `conf/.env` and pass
  `$OPS_NOTIFY_HEARTBEAT_URL`, keeping the push token out of a `ps`-visible
  command line.

  **The trap worth keeping:** Kuma's *"Default enabled"* applies to **new
  monitors only** — its own help text says so. Backfilling the existing 30 is
  a **separate** "Apply on all existing monitors" toggle, default off. Ticking
  only the first would have produced a provider that looked configured, tested
  green, and notified for nothing that already existed. Verified by counting
  `monitor_notification` rows (31) against active monitors (31) rather than
  trusting the checkbox. Live heartbeat confirmed end-to-end:
  `Operator Notify | status=1 | "nothing pending"`.

  Earlier notes in this file said "25 monitors"; the real count was 30 before
  the new one. Corrected here rather than left to propagate.

- **The operator notifier shares fate with the channel it reports on** —
  2026-07-29. Opened 07-28 during the #77 deploy: one revoked Gmail app
  password killed the briefing, the security digest, and operator-notify at
  once, so the handoff reporting "outbound ops email is dead" could not be
  delivered *because of the condition it was reporting*. Nothing was ever lost
  (a failed send holds the cursor and retries), but a dead notifier and a quiet
  bus looked identical.

  Resolved without the second credential the item was weighing. The escalation
  channel already existed and was already exercised daily — Uptime Kuma, which
  the sibling wake shim has pushed to since day one and which owns monitoring
  by doctrine. `operator-notify.sh` gained an optional
  `$OPS_NOTIFY_HEARTBEAT_URL` it pings `up`/`down` on every real run;
  monitoring, thresholds, and alert routing stay in the product that owns them.
  No Rust, no MCP surface, no new state, no failure counter — and no
  outage-only path that could rot unnoticed. Pinging is best-effort: an
  unreachable monitor logs and changes neither exit code nor cursor.
  Control-tested 12/12 across every exit path, including that cursor retry
  semantics are unchanged and that a dead monitor is not a new failure mode.
  Reasoning in `docs/operator-notify.md`.

  Scope boundary held deliberately: this covers a dead *channel*, not a dead
  *host*. If the box dies, monitor and notifier die together — that wants an
  off-box check and is a different problem.

  The two shapes the item rejected up front stay rejected, and neither is what
  shipped: a heartbeat whose absence a *human* must notice is the same failure
  with more steps, and a pre-flight credential check alerts down the broken
  path. A dead-man monitor differs on exactly that axis — the silence is
  noticed by a machine whose only job is noticing silence, on the first missed
  window.

- **Operator visibility, tier 1** — 2026-07-28, #77. The design session resolved it to a convention plus a cron with **zero lines of Rust**: agents reply into the existing thread addressed to the operator's slug, and a read-scoped machine token plus a cron polls that queue and mails a digest. Contract in `docs/operator-notify.md`, poller in `scripts/operator-notify.sh`. Deployment and token mint handed to CC-Cloud (`019fa9fe`).
- **Per-agent MCP tokens with server-bound `from_agent`** — 2026-07-28, #73 and #75, released in v4.2.0. Deployed 2026-07-21; minting completed fleet-wide 2026-07-24 (prod boots `agent tokens configured count=8`), binding control-tested per host, and revoked tokens verified dead by probing for 401 rather than assumed. Remaining residuals are ops on other lanes: CC-HSR's local install of the re-minted pair (`019f95df`) and demoting the old shared bearer to break-glass.
- **Case-insensitive agent matching on handoff queries** — 2026-07-17, #64. `bb9a25b` had already made the agent filters `ILIKE`; the residual `_`-wildcard over-match on the MCP list filters is closed with `LOWER() = LOWER()` exact matching, plus an integration test pinning that `agent_x` no longer matches `agentXx`.
- **External-user release sweep** — 2026-05-18, #56 and #57. Repo description and topics, GitHub Releases backfill, README lede and quick-start, GHCR multi-arch pipeline, standalone `docker-compose.example.yml`. The `v3.2.0` image is live and public at `ghcr.io/thek3nsai/ops-brain`.
- **Node 20 → Node 24 action versions** — bumped to explicit Node-24 majors across `.github/workflows/` ahead of GitHub's 2026-06-02 auto-force.
