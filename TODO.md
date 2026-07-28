# TODO

Open work only. Shipped history lives in `CHANGELOG.md`, doctrine and hard stops in `ROADMAP.md`, code and deploy footguns in `GOTCHAS.md`.

**Release-intent doctrine, still in force:** _"don't sharpen for optics; build for the 4 CCs."_ The external-user release sweep shipped 2026-05-18 (#56, #57). No further external-polish work without actual external signal — someone files an issue, asks for help, or otherwise shows up.

## Open

**The operator notifier shares fate with the channel it reports on (2026-07-28).**
Found during the #77 deploy itself. The ops Gmail app password had been revoked
on Google's side, which killed the daily briefing, the Logwatch security digest,
and operator-notify *simultaneously* — every outbound path runs through
`ops/bin/send-gmail.py`. The handoff reporting "outbound ops email is dead"
could not be delivered **because of the condition it was reporting**.

Failure is safe but silent. A failed send does not advance the cursor
(control-tested: rc=1, cursor stayed `<none>`), so items are retried rather than
dropped — nothing is lost. The problem is that a dead notifier and a quiet bus
look **identical** from the operator's side: no mail either way. For a component
whose entire job is "tell you when something needs you," failing into a state
indistinguishable from healthy is the one failure mode that actually matters.
It only surfaced this time because a human happened to be mid-deploy watching
the log.

Scope: host-side, not server-side. Delegating transport to
`$OPS_NOTIFY_MAIL_CMD` was deliberate and stays — mail belongs to the host that
already sends briefings. The fix lives in `scripts/operator-notify.sh` and the
ops layer. **No Rust, no new tool surface, no new MCP fields.**

Direction, not yet decided: the script *already knows* it failed — it logs
`send FAILED` and holds the cursor. The only missing piece is carrying that
knowledge somewhere that does not share Gmail's fate. Escalating to a second,
independent channel after N consecutive failures keeps the normal path
unchanged (no added noise) and exercises the fallback exactly when the primary
is dead. Weigh that against the cost of a second credential that can also
expire quietly.

Two shapes to reject up front: a heartbeat whose *absence* you're supposed to
notice (same class of problem — it asks a human to detect silence), and a
pre-flight credential check (useful, but its alert path is the broken one).

Both threads that previously sat here closed on 2026-07-28.

## Don't re-propose without new evidence

Deliberate decisions with their reasons. If real friction ever shows up, re-open the question from first principles — don't resurrect the design.

**Four-lens audit declines (2026-07-17, v4.1.0, PRs #64/#65/#67).** Every finding was fixed, tested, or consciously declined. The declines:

- **Verbatim DB error strings to callers** — every caller is one of our own authenticated agents; informative errors beat sanitization theater here.
- **`build_client_lookup` caching / backfill batching / embedding retry-backoff** — real, but irrelevant at 4-agent scale. The perf audit put these on its own "theater" list.
- **Unified error enum** — the current boundary-appropriate typing (`sqlx::Error` → `CallToolResult` / `(StatusCode, String)`) is deliberate, not mess.
- **Briefing cosmetic nits** — a section is omitted when a LIMIT-20 page holds none of a counted status; the unscoped `_note` yields to the rarer embedding note on multi-table. Edge-case cosmetic, flagged in #67's body.

**Automation backbone, phase 3 (2026-07-17, joint design with CC-HSR).** Phases 1+2 shipped; producer contract in `docs/machine-callers.md`. Deliberately not built: server-side recurrence (producers' own schedulers plus `dedupe_key` idempotency cover it), structured handoff columns (the versioned `context` convention instead — promote fields only if a pilot bleeds from prose-parsing), and webhooks-out (poll `GET /api/pending`; an additive upgrade if sub-minute latency ever becomes real pain).

**Operator visibility, tier 2 (2026-07-28).** A log or view of everything happening headless. The handoffs table already *is* the log — `origin`, `status`, `repeat_count`, threading, timestamps. The only real gap is that `updated_at` is destructive, so there's no `accepted_at` and "how long did this sit on a human" isn't answerable. That's a schema change in service of a metric nothing is bleeding from. If friction shows up, the cheapest next step is a section in the briefing that already gets read — not a web view, not event sourcing. Full reasoning in `docs/operator-notify.md`.

## Closed

- **Operator visibility, tier 1** — 2026-07-28, #77. The design session resolved it to a convention plus a cron with **zero lines of Rust**: agents reply into the existing thread addressed to the operator's slug, and a read-scoped machine token plus a cron polls that queue and mails a digest. Contract in `docs/operator-notify.md`, poller in `scripts/operator-notify.sh`. Deployment and token mint handed to CC-Cloud (`019fa9fe`).
- **Per-agent MCP tokens with server-bound `from_agent`** — 2026-07-28, #73 and #75, released in v4.2.0. Deployed 2026-07-21; minting completed fleet-wide 2026-07-24 (prod boots `agent tokens configured count=8`), binding control-tested per host, and revoked tokens verified dead by probing for 401 rather than assumed. Remaining residuals are ops on other lanes: CC-HSR's local install of the re-minted pair (`019f95df`) and demoting the old shared bearer to break-glass.
- **Case-insensitive agent matching on handoff queries** — 2026-07-17, #64. `bb9a25b` had already made the agent filters `ILIKE`; the residual `_`-wildcard over-match on the MCP list filters is closed with `LOWER() = LOWER()` exact matching, plus an integration test pinning that `agent_x` no longer matches `agentXx`.
- **External-user release sweep** — 2026-05-18, #56 and #57. Repo description and topics, GitHub Releases backfill, README lede and quick-start, GHCR multi-arch pipeline, standalone `docker-compose.example.yml`. The `v3.2.0` image is live and public at `ghcr.io/thek3nsai/ops-brain`.
- **Node 20 → Node 24 action versions** — bumped to explicit Node-24 majors across `.github/workflows/` ahead of GitHub's 2026-06-02 auto-force.
