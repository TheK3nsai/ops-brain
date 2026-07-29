# Operator notification

The bus runs unattended. Machine producers file handoffs, wake shims spawn
headless sessions, and work gets accepted and completed with no human in the
loop. That is the point — but it means a handoff that lands *on the operator*
is invisible unless they happen to read the thread.

This is the convention and the delivery path for "the bus is blocked on you".

## The convention: the operator is just another agent

**When you are blocked on a human, file the handoff addressed to them.**

```
create_handoff(
  to_agent:    "Operator",          # your operator's slug
  in_reply_to: "<the thread you're already working>",
  category:    "action",
  title:       "Blocked: <what you need from them>",
  body:        "<what you did, exactly what you need, what unblocks it>"
)
```

That is the entire mechanism. No new tool, no new field, no flag. `to_agent`
is free-form and `GET /api/pending` does not care that the agent it is polling
for is a person.

Two rules make it work:

- **Reply into the existing thread; do not open a parallel one.** The blocked
  state belongs to the work it is blocking. A second standalone handoff is
  duplicate truth — two objects tracking one state, which then disagree.
- **`category: "action"`.** The poll filters on it. A `notify` handoff
  addressed to the operator will never reach them through this path (by
  design — FYI broadcasts are what the briefing is for).

**Closing the loop is the filer's job.** The operator does not call
`accept_handoff`. An item addressed to them stays open until the agent that
was blocked completes it once unblocked. If you file one, you own completing
it.

## Delivery: a dumb cron, not a feature

`scripts/operator-notify.sh` polls `GET /api/pending?agent=<operator-slug>` with a
read-scoped machine token and renders a digest of anything new since its last
run. It is the wake-shim pattern pointed at a human, and it needs **zero
ops-brain code** — the endpoint, the token scoping, and the `since` cursor all
already exist.

Token to mint (`OPS_BRAIN_MACHINE_TOKENS`):

```json
{
  "token": "<32+ char minted secret>",
  "from_agent": "Operator-Notify",
  "agents": ["Operator"],
  "scopes": ["read"]
}
```

`read` only, one agent in the allowlist. This credential cannot file anything,
cannot reach `/mcp`, and cannot poll any other agent's queue. `from_agent` is
required by the token schema but is never used — nothing is ever filed with it.

Cron (every 15–30 min is plenty; this is "before end of day", not an alert):

```cron
*/20 * * * * OPS_BRAIN_URL='https://ops.example.com' OPS_NOTIFY_MAIL_CMD='<the mailer>' /path/to/operator-notify.sh
```

The script renders the digest and pipes it to `$OPS_NOTIFY_MAIL_CMD` on stdin;
with that unset it writes to stdout. Sending is deliberately delegated — mail
credentials and transport belong to the host that already sends the briefings,
not to this script. See `--help` for the full environment.

Failure behavior: a failed poll or a failed send **does not advance the
cursor**, so the next run retries the same items rather than dropping them. The
script is silent and exits 0 while the token file is absent, so the cron can be
installed before the mint lands.

## The notifier must not share fate with the channel it reports on

Nothing is ever lost when a send fails — the cursor is held and the items are
retried. The problem is narrower and worse: **a dead mailer and a quiet bus
look identical from the operator's side.** No mail either way. For a component
whose whole job is "tell you when something needs you," failing into a state
indistinguishable from healthy is the one failure mode that actually matters.

This is not hypothetical. On 2026-07-28 the mail credential behind
`$OPS_NOTIFY_MAIL_CMD` was revoked provider-side, taking the daily briefing,
the security digest, and this notifier down together — every outbound path ran
through one credential. The handoff reporting *"outbound ops email is dead"*
could not be delivered **because of the condition it was reporting**. It
surfaced only because a human happened to be watching a log during a deploy.

The fix is not a second mailer. Set `$OPS_NOTIFY_HEARTBEAT_URL` to a dead-man
monitor — an Uptime Kuma push monitor, or anything else accepting
`?status=up|down&msg=...`:

```cron
*/20 * * * * . "$HOME/ops/conf/.env"; \
             OPS_BRAIN_URL='https://ops.example.com' \
             OPS_NOTIFY_MAIL_CMD='<the mailer>' \
             OPS_NOTIFY_HEARTBEAT_URL="$KUMA_OPERATOR_NOTIFY_PUSH" \
             /path/to/operator-notify.sh
```

Keep the URL in a mode-600 env file rather than inline in the crontab: its last
path segment is a push token, and a crontab command line is visible in `ps` to
every local user. `--status` elides that segment for the same reason.

Every real run reports: `up` when it polled cleanly (with or without items to
send), `down` with a reason when the poll fails, the token is unusable, or the
send fails. Miss enough runs entirely — cron dead, host down, script removed —
and the monitor goes stale on its own. `--dry-run` never reports, so testing
cannot flip a live monitor.

Three properties make this the right shape rather than a second credential:

- **The silence is noticed by a machine whose only job is noticing silence.**
  A heartbeat a *human* has to miss is the same failure again with more steps.
- **The escalation path is exercised continuously.** A fallback used only
  during outages is a fallback that is broken when you finally need it. The
  monitoring system is running and alerting on other monitors every day, so it
  cannot rot unnoticed the way an idle backup credential does.
- **No new state and no threshold.** The script does not count consecutive
  failures; it reports each run truthfully and lets the monitor's own retry and
  resend settings decide when a human is worth waking. Alerting policy belongs
  to the monitoring system, not to this script.

Pinging is best-effort by construction: an unreachable monitor logs a line and
changes neither the exit code nor the cursor. The channel that reports failures
must never become a new way to fail.

**This covers a dead channel, not a dead host.** If the box goes down, the
monitor and the notifier go with it. Whole-host outage is a different problem
and wants an off-box check; do not conflate the two.

For this to mean anything, **the monitoring system needs a notification
channel that does not depend on the same credential as the mailer.** A dead-man
monitor wired back into the mail path that just died buys nothing.

## Division of labor with the briefing

| Surface | Answers |
|---|---|
| This notifier | "Something new needs you." Cursor-based; fires once per new or bumped item. |
| Daily briefing | "Here is everything still open." Already cron-mailed, already fleet-wide. |

A long-blocked item goes quiet on the notifier after its first alert — that is
intentional. Re-nagging every 20 minutes trains you to ignore it, and the
standing list of what is still open is what the briefing is already for.

## What this deliberately is not

No dashboard, no web view, no event log, no transition timestamps. The
handoffs table already *is* the log — `origin`, `status`, `repeat_count`,
threading, and timestamps are all there. What was missing was a *push* for the
one state that needs a human, and that is what this is.

A richer history view (notably `accepted_at`, which does not exist today —
`updated_at` is overwritten on every touch) is a schema change in service of a
metric nothing is currently bleeding from. If real friction shows up, the
cheapest next step is a section in the briefing that already gets read, not a
new surface. See `ROADMAP.md` on measurement as ceremony.
