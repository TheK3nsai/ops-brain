# Machine callers

Non-interactive producers — monitors, cron sweeps, scripts — can file handoffs
and poll an agent's open queue over plain HTTP, without an MCP session. This
is the ingestion path for "routine work runs unattended; findings arrive as
pre-filed work items; agents pick them up the same way they pick up
human-filed work."

Two endpoints, and only these two, are reachable with a machine token:

| Endpoint | Scope | Purpose |
|---|---|---|
| `POST /api/handoff` | `create` | File a handoff (idempotent via `dedupe_key`) |
| `GET /api/pending` | `read` | Poll an agent's open action queue (wake shims) |

Everything else — `/mcp`, `/api/briefing` — rejects machine tokens with 403.

## Scope boundaries (read first)

- **ops-brain never owns execution timing.** Recurrence lives on the
  producer's own scheduler (cron, systemd timers, Task Scheduler). "Weekly
  check X" is a cron line that POSTs here — not a server-side definition.
- **Dead-man detection is out of scope.** If a producer's scheduler dies, no
  handoff is filed and silence looks green. Producers must self-monitor their
  own liveness (e.g. a heartbeat/push monitor on the sweep itself).
- **Context and body are PHI/PII-free by contract.** Pointers, counts,
  hostnames, slugs — never record contents, personal data, or file payloads.
  Evidence stays on the producer's infrastructure; the bus carries a
  *reference* to it.

## Auth: machine tokens

Configured server-side via `OPS_BRAIN_MACHINE_TOKENS` (JSON array):

```json
[
  {
    "token": "<32+ char minted secret>",
    "from_agent": "Example-Host1",
    "client": "example",
    "agents": ["CC-Example"],
    "scopes": ["create", "read"]
  }
]
```

- `token` — the bearer secret, sent as `Authorization: Bearer <token>`.
  Minimum 32 chars; must differ from the main auth token. Mint one per
  machine (not per script — `context.source` carries script identity).
- `from_agent` — stamped on every handoff this token files. Callers cannot
  override it; supplying a different `from_agent` in the request body is a
  400. The token IS the identity.
- `client` — informational scope label, recorded in server logs.
- `agents` — the routing allowlist: which agents this token may file
  handoffs **to** and poll pending queues **for**. Case-insensitive exact
  match, no wildcard, must be non-empty.
- `scopes` — `create` and/or `read`, matching the endpoint table above.

Never put the agents' full-surface bearer in a scheduled task. That token
reaches the entire MCP surface; a machine token bounds theft blast-radius to
"can file work items to its own lane and read that lane's queue titles."

## `POST /api/handoff`

```json
{
  "to_agent": "CC-Example",
  "title": "[auto] Disk usage WARN: /var 91% (threshold 90%)",
  "body": "Markdown body: what was measured, value vs threshold, runbook pointer.",
  "priority": "normal",
  "category": "action",
  "dedupe_key": "example-host1-var-disk",
  "context": { "...": "see convention below" }
}
```

- `to_agent` — required; must be in the token's `agents` allowlist. Machine
  callers cannot file open (unaddressed) handoffs.
- `priority` — `low` | `normal` | `high` | `critical` (default `normal`).
  Priority lives HERE, not in context — one source of truth.
- `category` — `action` (default) or `notify`.
- `origin` is stamped `machine` server-side on everything filed through this
  endpoint. It is not a request field.

**Responses**

- `201` — filed fresh: `{"id": "...", "status": "pending", "deduplicated":
  false, "repeat_count": 0, "warnings": [...]}`
- `200` — suppressed into an existing open handoff with the same
  `dedupe_key`: `{"id": "<existing id>", "deduplicated": true,
  "repeat_count": N, ...}`. The existing row's `updated_at` is bumped and
  `repeat_count` incremented, so consumers can tell "still firing after N
  runs" from "filed once and went quiet."
- `400` / `403` — validation / routing-scope failures. `warnings` are
  advisory only and never block a filing.

### Dedupe semantics

`dedupe_key` is caller-chosen, per-*check* (not per-run): e.g.
`example-host1-var-disk`, not `...-2026-07-17`. Uniqueness is enforced only
against **open** handoffs (pending/accepted), and is scoped **per recipient**
(`to_agent`, case-insensitive): the same key filed to two different agents
files two independent handoffs — suppression only applies to repeat filings
targeting the same agent. The lifecycle:

1. Check fails → POST files a fresh handoff (201).
2. Check fails again next run → POST suppresses into the open handoff,
   bumps `repeat_count` (200).
3. An agent fixes the issue and completes the handoff → the key is released.
4. Check fails again later → POST files a fresh handoff (201).

Allowed key characters: `a-zA-Z0-9 . - _ /`, max 200 chars.

**Suppression freezes the payload.** A suppressed POST bumps `repeat_count`
and `updated_at` only — title, body, priority, and context stay as first
filed. If a condition can escalate in severity (WARN → FAIL), either tier the
key (`...-warn` / `...-fail` file as separate handoffs) or accept that the
open handoff reflects first observation and the agent re-measures on pickup.

### Context convention v1

`context` must be a JSON object when present. Unknown keys are kept and
warned about in the response — the convention is a contract, not a cage.
Mirror this table in your own infrastructure's docs so producers don't
depend on this repo to stay conformant.

```json
{
  "v": 1,
  "source": "host1/disk-sweep",
  "check": "disk/var-usage",
  "verdict": "WARN",
  "observed_at": "2026-07-17T09:00:00Z",
  "evidence_ref": "/var/log/disk-sweep/2026-07-17.json",
  "metrics": { "used_pct": 91, "threshold_pct": 90 }
}
```

| Field | Meaning |
|---|---|
| `v` | Convention version (currently `1`). Anchors future migrations. |
| `source` | `machine/script` id — disambiguates producers under a per-machine token. |
| `check` | Stable slug for the specific check; typically the `dedupe_key` stem. |
| `verdict` | `PASS` \| `WARN` \| `FAIL` \| `UNKNOWN`. Convention: file only WARN/FAIL/UNKNOWN — PASS belongs in local reports, not on the bus. |
| `observed_at` | Measurement time (ISO-8601) — distinct from the row's `created_at`. |
| `evidence_ref` | **Pointer** to evidence on the producer's own infrastructure (local path, repo-relative report). Never a payload; there is deliberately no `evidence_url` — evidence does not leave the producer's boundary. |
| `metrics` | Small flat numeric map (value vs threshold). Optional. |

## `GET /api/pending?agent=CC-Example&since=2026-07-17T12:00:00Z&limit=50`

Open **action** handoffs addressed to `agent` (which must be in a machine
token's `agents` allowlist). `since` filters on `updated_at` — dedupe bumps
re-surface past the cursor, so a still-firing monitor shows up even if the
handoff predates it. `limit` defaults to 50 (max 200).

```json
{
  "count": 1,
  "items": [
    {
      "id": "0198...",
      "title": "[auto] Disk usage WARN: /var 91% (threshold 90%)",
      "status": "pending",
      "priority": "normal",
      "category": "action",
      "origin": "machine",
      "from_agent": "Example-Host1",
      "dedupe_key": "example-host1-var-disk",
      "repeat_count": 2,
      "created_at": "2026-07-16T10:10:00Z",
      "updated_at": "2026-07-17T10:10:00Z"
    }
  ]
}
```

Items are deliberately body-free so a short-interval poll stays a few hundred
bytes; the woken agent fetches full bodies over MCP (`check_in` /
`list_handoffs`). A typical wake shim: poll every 5 minutes with a persisted
`since` cursor; when `count > 0`, kick off a local headless agent run and
advance the cursor once the run has picked the work up.

## Wiring a new producer (checklist)

1. Mint a 32+ char secret; add its binding to `OPS_BRAIN_MACHINE_TOKENS`
   (both `.env` and the prod compose enumerate it) and redeploy.
2. Store the secret on the producer machine with least privilege — never in
   the repo, never the main bearer.
3. Schedule the check locally; POST only on WARN/FAIL/UNKNOWN with a
   per-check `dedupe_key`.
4. Add a liveness heartbeat for the check itself (dead-man is yours).
5. Mirror the context convention in your infra's own docs.
