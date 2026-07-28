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
