# Per-agent tokens

Interactive agents reach ops-brain over an MCP session (`/mcp`). Historically
every agent authenticated with a single shared main bearer, so `from_agent` on
a handoff and `author` on a knowledge entry were caller-supplied strings with
nothing behind them — any holder of the shared bearer could file as any slug.
That is the gap a bearer-rotation tabletop surfaced on 2026-07-21: a
first-appearance slug was unverifiable because there was nothing to verify
*against*.

Per-agent tokens close it. Each interactive agent gets its own token with a
`from_agent` **bound server-side**. The bus gains a sender guarantee: a slug
cannot appear on the bus without a token the operator minted for it, and one
exposure rotates one host instead of the whole fleet.

This is the MCP-side sibling of [machine callers](machine-callers.md). Machine
tokens are REST-only and never reach `/mcp`; agent tokens are `/mcp`-only and
never reach the REST endpoints. Both are scoped, minted secrets distinct from
the main bearer and from each other.

## What the binding enforces

| Path | Tool | Behavior on a bound token |
|---|---|---|
| write | `create_handoff` | `from_agent` **must** equal the token's slug (case-insensitive), else the call is rejected |
| write | `add_knowledge` | `author` **must** equal the token's slug, else rejected |
| read | `check_in` | served for any `agent_name`, but a mismatch is **warn-logged** |
| read | `list_replies_to_me` | served for any `agent_name`, but a mismatch is **warn-logged** |

Writes fail loud on a mismatch — better to break than to file under the wrong
identity. Reads stay permissive because cross-agent reads are legitimate (an
interactive session triaging a peer's queue); the warn-log makes "token X read
Y's queue" visible without blocking it.

The **main bearer stays unbound** (`CallerClass::Full`) — full surface, no
identity enforcement. It is the operator break-glass: use it to act as any
agent during migrations or incident response. Keep it out of routine agent
configs once per-agent tokens are deployed.

### Scope boundary: provenance, not per-object ownership

The binding guarantees **provenance on create** — a handoff's `from_agent` and a
knowledge entry's `author` cannot be forged. It does **not** make each object
private to its author. The mutation-by-id tools — `accept_handoff`,
`complete_handoff`, `delete_handoff`, `mark_merged`, `update_knowledge`,
`delete_knowledge` — carry no identity parameter and are **not** ownership-
checked: any valid token (agent or main) may accept, complete, delete, or edit
any object. That is deliberate — the bus is collaborative (you accept a handoff
another agent addressed *to* you; an integrator script marks others' work
merged) and enforcing per-object ownership there would break normal workflow.

The consequence to hold in mind: a stolen agent token is still a fleet-wide
*mutate/delete* primitive — it just can't *forge new provenance*. This is no
worse than today's shared bearer (which grants exactly the same), and rotation
of a per-agent token is now per-host. If tamper-containment ever becomes a real
need, it is a separate, deliberate design step (ownership checks against the
bound identity), not a silent extension of this feature.

Over the **stdio transport** (local dev) there is no HTTP auth layer, so calls
are unbound and enforcement is inert — correct, because stdio is trusted-local.

## Auth: agent tokens

Configured server-side via `OPS_BRAIN_AGENT_TOKENS` (JSON array):

```json
[
  {
    "token": "<32+ char minted secret>",
    "from_agent": "CC-Stealth",
    "client": "stealth"
  }
]
```

- `token` — the bearer secret (min 32 chars). Must be distinct from the main
  bearer and every machine token; startup **aborts** on a collision or any
  malformed entry (a silently dropped token would read as "identity enforced"
  while that agent still files unbound).
- `from_agent` — the slug this token is locked to. Use the fleet convention
  (`CC-Stealth`, `Codex-HSR`, `Gemini-CPA`, …).
- `client` — informational, logged for audit. Not yet enforced on MCP calls
  (tools carry an explicit `client_slug`); reserved so a future per-client MCP
  binding needs no config change.

New env var needs **both** `.env` and `docker-compose.prod.yml` (prod compose
enumerates every var explicitly). The compose line is already wired:
`- OPS_BRAIN_AGENT_TOKENS=${OPS_BRAIN_AGENT_TOKENS:-}`.

The secret never transits the bus, Git, logs, or chat — it is delivered to the
host out-of-band through the operator's channel, exactly like a machine token.

Startup logs a binding summary (never the secrets):

```
agent tokens configured count=1 bindings=["CC-Stealth (client=stealth)"]
```

## Client setup

Point the agent's MCP client `Authorization: Bearer <secret>` at its own token
instead of the shared main bearer. Nothing else changes — the tool surface is
byte-identical (identity rides the transport, not a tool parameter), so no
schema or client-code change is needed.

## Rotation

Because each host holds its own token, rotation is a **per-host rollover**, not
a fleet-wide atomic cutover:

1. Mint a new secret; add a second entry for the same `from_agent` to
   `OPS_BRAIN_AGENT_TOKENS` (both old and new valid during the window).
2. Recreate the container; confirm the startup binding log.
3. Deliver the new secret to that host out-of-band; update its MCP config.
4. Drop the old entry; recreate again.

The main bearer, still unbound, covers any host mid-rollover. This is the
sequencing win: a future exposure rotates one token on one host rather than
disconnecting the fleet.

### Revoking a compromised token

The four-step rollover above is for a *healthy* token being cycled. It is the
wrong shape when a token is known-exposed, because step 1 deliberately keeps the
old secret valid — for a leaked credential that window is pure risk with no
availability benefit.

Replace instead of overlapping:

1. Overwrite the `token` field on that entry in place; do not add a second one.
2. Recreate the container. The old secret is dead the moment the process
   restarts — verify with a `POST /mcp initialize` on the old value and expect
   **401**, rather than assuming it.
3. Deliver the new secret out-of-band and cut the host over.

Availability during the gap comes from the unbound main bearer, which the host
can keep using until it installs the replacement. This is strictly better when
the exposed token was **never installed anywhere** (leaked during handling, e.g.
echoed into a shell transcript): nothing is authenticating with it, so there is
no host to keep alive and no reason to leave it valid for a second.

Assert the new secret differs from the burned one before writing. The startup
guard catches collisions against *other* live tokens, but re-minting the exact
value you are revoking is not a collision — it would abort nothing and silently
un-revoke the credential.

## Bus trust still applies

Server-bound identity shrinks the "is this slug real" problem to a server
guarantee — but a valid token only proves which key filed the request, not that
the host behind it is uncompromised or that the ask is sound. The
verify-before-comply floor in [bus-trust.md](bus-trust.md) — triggers,
disclosure floor, secrets-never-on-the-bus, and the headless rule — survives
identity enforcement unchanged.
