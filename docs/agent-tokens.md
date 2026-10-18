# Per-agent tokens

Interactive agents reach ops-brain over MCP (`/mcp`). Without per-agent
tokens, `from_agent` and `author` are caller-supplied strings: any holder of the
shared main bearer can file as any slug.

Per-agent tokens close it. Each interactive agent gets its own token with a
`from_agent` **bound server-side**. The bus gains a sender guarantee: a slug
cannot appear on the bus without a token the operator minted for it, and one
exposure rotates one host instead of the whole fleet.

This is the interactive sibling of [machine callers](machine-callers.md).
Machine tokens are REST-only and never reach `/mcp`; agent tokens reach
`/mcp` only, never REST. Both are minted secrets distinct from
the main bearer and from each other.

## What the binding enforces

| Path | Tool | Behavior on a bound token |
|---|---|---|
| write | `create_handoff` | `from_agent` must match the token's slug or configured `-*` family, else rejected |
| write | `add_knowledge` | `author` must match the token's slug or family, else rejected |
| claim | `accept_handoff` | records `accepted_by`; family credentials require a concrete matching `agent_name` |
| read | `check_in` | served for any `agent_name`, but a mismatch is **warn-logged** |
| read | `list_replies_to_me` | served for any `agent_name`, but a mismatch is **warn-logged** |

Writes fail loud on a mismatch — better to break than to file under the wrong
identity. Reads stay permissive because cross-agent reads are legitimate (an
interactive session triaging a peer's queue); the warn-log makes "token X read
Y's queue" visible without blocking it.

The **main bearer stays unbound** (`CallerClass::Full`) — full durable surface,
no identity enforcement. It is the operator break-glass for migrations or
incident response. It lives **on the server only**. Once per-agent tokens are
deployed, no agent host keeps a copy, not even a dormant one for emergencies.

### Scope boundary: provenance, not per-object ownership

The binding guarantees **provenance on creation and claim** — a handoff's
`from_agent` and `accepted_by`, and a knowledge entry's `author`, cannot be
forged outside the credential's bound slug or family. Object mutations remain collaborative:
`accept_handoff`, `complete_handoff`, `delete_handoff`, `mark_merged`,
`update_knowledge` and `delete_knowledge` are not ownership-checked. An
unrestricted token may act on any object; restricted tokens retain the
visibility and author boundaries below. That is deliberate (you accept a handoff
another agent addressed *to* you; an integrator script marks others' work
merged) and enforcing per-object ownership there would break normal workflow.

The consequence to hold in mind: a stolen unrestricted agent token is still a fleet-wide
*mutate/delete* primitive — it just can't *forge new provenance*. This is no
worse than a shared bearer (which grants exactly the same), and rotation
of a per-agent token is per-host. If tamper-containment ever becomes a real
need, it is a separate, deliberate design step (ownership checks against the
bound identity), not a silent extension of this feature.

Over the **stdio transport** (local dev) there is no HTTP auth layer, so durable
write bindings are inert. Unbound callers can supply `agent_name` when claiming
a handoff; broadcasts require it, while addressed handoffs default to their
recipient. Preexisting accepted rows have no recorded acceptor and are not
backfilled by guessing. Accepted broadcasts with a known acceptor stay in
that seat's check-in, wake queue and briefing.

Shared credentials share provenance across all workers that use them; see
[client setup](clients.md).

## Auth: agent tokens

Configured server-side via `OPS_BRAIN_AGENT_TOKENS` (JSON array):

```json
[
  {
    "token": "<32+ char minted secret>",
    "from_agent": "Claude-Laptop",
    "client": "acme"
  }
]
```

- `token` — the bearer secret (min 32 chars). Must be distinct from the main
  bearer and every machine token; startup **aborts** on a collision or any
  malformed entry (a silently dropped token would read as "identity enforced"
  while that agent still files unbound).
- `from_agent` — the slug this token is locked to. Use the fleet convention
  (`Claude-Laptop`, `Codex-Buildbox`, …).
  A trailing `-*` binds a slug family instead: `Grok-Bot-*` may write as
  `Grok-Bot` or any `Grok-Bot-<Name>`, and an `exclude_client_data`
  boundary covers the whole family. Use it only when several agents must
  share one credential (Grok Bots ride one MCP connector per account; each
  bot files as `Grok-Bot-<BotName>`, e.g. `Grok-Bot-Builder`). The
  family members can impersonate each other, so it is provenance per
  credential, not per bot. Each member writes as its own slug
  (`from_agent`, `author`), not the bare base, and passes it as `agent_name`
  to `accept_handoff`, which rejects a family claim without one.
- `client` — informational, logged for audit. Not yet enforced on MCP calls
  (tools carry an explicit `client_slug`); reserved so a future per-client MCP
  binding needs no config change.
- `exclude_client_data` — optional, default `false`. When `true`, the token
  is kept out of client data entirely. See "Keeping an agent out of client
  data" below.

New env var needs **both** `.env` and `docker-compose.prod.yml` (prod compose
enumerates every var explicitly). The compose line is already wired:
`- OPS_BRAIN_AGENT_TOKENS=${OPS_BRAIN_AGENT_TOKENS:-}`.

The secret never transits the bus, Git, logs, or chat — it is delivered to the
host out-of-band through the operator's channel, exactly like a machine token.

Startup logs a binding summary (never the secrets):

```
agent tokens configured count=1 bindings=["Claude-Laptop (client=acme)"]
```

## Keeping an agent out of client data

`"exclude_client_data": true` is for an agent you want on the bus but not near
client data, typically one whose vendor hasn't cleared a data-handling review.
Without the flag, the client-scope guard is only a disclosure gate: any token
can read client-scoped knowledge by acknowledging it, and every token can read
all handoffs. A convention of "don't send it client data" binds the senders,
not the reader. The flag moves that boundary into the server.

A flagged token:

| Surface | Behavior |
|---|---|
| knowledge reads (`search_bus`, browse, duplicate detection on `add_knowledge`) | global rows only (`client_id IS NULL`). A client row stays out even when it is `cross_client_safe`. |
| handoff reads (`search_bus` handoffs, `list_handoffs`, `check_in`, `list_replies_to_me`) | only rows whose sender or recipient matches the token's slug or family. Broadcasts are excluded, since anyone may have written them for anyone. |
| `check_in`, `list_replies_to_me` | its bound slug or a concrete family member only; other queues are rejected |
| by-id tools (`get_handoff`, `accept_handoff`, `complete_handoff`, `mark_merged`, `delete_handoff`, `update_knowledge`, `delete_knowledge`) | a row outside the boundary returns **not found**, so the reply doesn't confirm it exists |
| `update_knowledge`, `delete_knowledge` on a global row | allowed only on rows the token authored, so it can't rewrite rules other agents rely on |
| `create_handoff` with `in_reply_to` | the parent must be inside the boundary |
| `create_handoff` to a never-seen recipient | no first-recipient `_warning`: that check scans every handoff, so its answer would describe traffic outside the boundary |
| `add_knowledge`, `search_bus` with `client_slug` | rejected |

The filters run in the SQL, inside each search leg, so nothing outside the
boundary reaches the response. That includes the cross-client withheld
notices, which name the owning client. `search_bus` responses carry
`_restricted` so the agent can tell an empty result from a filtered one. One
recall caveat: an HNSW vector scan filters after it picks its candidates, so a
restricted semantic search can come back short when most near neighbours are
outside the boundary. FTS legs are unaffected.

### Before enabling it

- **Bind a slug with no history.** The handoff filter matches on the slug
  string, not on the token. Rebinding a retired slug (say a former test seat)
  makes everything ever written to or from it readable. Check first:
  `SELECT count(*) FROM handoffs WHERE lower(from_agent) = lower('<slug>') OR lower(to_agent) = lower('<slug>');`
- **Audit global knowledge.** The boundary is `client_id IS NULL`, so it
  trusts every global row to be client-free. A row filed without
  `client_slug` that carries client detail (hostnames, paths, staff) is
  visible. Browse `search_bus` with `query: "*"` and re-scope or delete those
  rows first. Deleting a client can't add to that set: `knowledge.client_id`
  is `ON DELETE RESTRICT`, so the delete fails while the client owns rows.
- **Confirm it took.** Unknown keys in an `OPS_BRAIN_AGENT_TOKENS` entry abort
  startup, so a misspelled flag can't silently default to off. After
  recreating the container, the binding summary must show
  `<slug> (client=…, exclude_client_data=true)`.

### What it does not do

- It doesn't stop *other* agents putting client data into a handoff they
  address to the flagged agent, or answering one it sends asking for client
  data. The server doesn't tell senders that a recipient is restricted, so
  every sender still has to follow the rule. The flag guarantees only that the
  agent can't go and fetch what wasn't sent to it.
- A reply into the agent's thread with no `to_agent` is invisible to it,
  because broadcasts are outside the boundary. Address replies to it
  explicitly.
- It isn't tenant isolation. Across a real trust boundary, a separate
  deployment is still the stronger answer.

## Client setup

Point the agent's MCP client `Authorization: Bearer <secret>` at its own token
instead of the shared main bearer. Identity rides the transport for durable
writes.

## Adding an agent to the bus (checklist)

The whole path for a new agent on a new or existing host. Steps 1–3 are
operator-only and happen on the server; nothing else needs the operator.

1. **Pick the slug** — `Claude-<Host>` / `Codex-<Host>`. One token per agent, so a
   host running both clients gets two.
2. **Mint and bind** — 32+ char secret, new entry in `OPS_BRAIN_AGENT_TOKENS`
   (`.env` on the server), recreate the container, confirm the slug in the
   startup binding log.
3. **Deliver the secret out-of-band** to the host. Never over the bus, Git,
   logs, or chat.
4. **Configure the MCP client** on the host: HTTP transport, the deployment's
   `/mcp` URL, `Authorization: Bearer <that agent's token>` (see "Client
   setup" above). Store the secret in a file only that user can read; don't
   export it from a shell rc file.
5. **Prove the binding with a positive and a negative control** — from the new
   agent, a write filed as its own slug succeeds; the same write filed as any
   other slug is rejected. A read proves nothing (reads are unbound).
6. **Tell the agent three things in its local instructions**, and nothing
   more: its own slug, the operator's slug (where blocked-on-a-human replies
   go), and which sibling agents exist. Workflow conventions arrive with the
   server's MCP instructions — don't copy them into local files.
7. **Optional:** a wake shim so handoffs reach an agent nobody is sitting at
   (`machine-callers.md`, needs a machine token).

## Rotation

Because each host holds its own token, rotation is a **per-host rollover**, not
a fleet-wide atomic cutover:

1. Mint a new secret; add a second entry for the same `from_agent` to
   `OPS_BRAIN_AGENT_TOKENS` (both old and new valid during the window).
2. Recreate the container; confirm the startup binding log.
3. Deliver the new secret to that host out-of-band; update its MCP config.
4. Drop the old entry; recreate again.

The overlap in step 1 is what keeps the host on the bus mid-rollover. The old
token stays valid until the new one is installed, so nothing leans on the main
bearer. This is the sequencing win: a future exposure rotates one token on one
host rather than disconnecting the fleet.

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

There is no bridge for the gap: the host is off-bus from the recreate until
it installs the replacement. Don't pre-stage the main bearer to cover it. A
separate machine token keeps the REST lane (pending polls, producer writes)
alive, but a wake it triggers runs without bus tools, so expect wake-lane
failure alerts during the gap. If the exposed token was never installed
anywhere, there is no gap at all.

Assert the new secret differs from the burned one before writing. The startup
guard only catches collisions with *other* live tokens; re-minting the revoked
value would silently un-revoke it.

### When the replacement is gated

If the new secret must not be installed until some condition holds (for
example, the host still writes its environment to disk where the old one
leaked):

- **Withhold the mint; don't mint and attach the condition as an instruction.**
  Remove the entry (N → N−1) and leave that lane off-bus until the gate clears.
- **Announce the withhold before the recreate**, stating which lane is off-bus
  and what releases it. A multi-identity host looks healthy without it.
- **Whoever carries the credential carries the condition**, out-of-band, not
  on the bus.
- **Release on posted evidence**, not on a status report or availability
  pressure.
- **Verify the withheld state:** assert the entry's fingerprint before
  removing it, recreate with `-f docker-compose.prod.yml`, and confirm the
  `agent tokens configured` line shows `count = N−1` with the slug absent and
  every other binding intact. Probe the burned value for 401 (against a no-auth
  control) and one untouched lane for 200.

A gate discovered *after* the mint is a leaked credential: use *Revoking a
compromised token* and gate the re-install instead.

### What "revoked" does and does not mean

Revocation is authoritative about the server only: after the restart the old
secret authenticates nowhere. It says nothing about copies. Compressed and
content-addressed stores (gzip, git objects, DB dumps, restic/borg) and
whole-disk images (provider backups, hypervisor checkpoints) keep values a
plaintext sweep can't see. Write the close-out as **"dead server-side now; gone
from images and archives in ≤N days"** (N = longest retention), never "rotated
and swept clean". Scope any credential that can read those images to read-only.

## Bus trust still applies

Server-bound identity shrinks the "is this slug real" problem to a server
guarantee — but a valid token only proves which key filed the request, not that
the host behind it is uncompromised or that the ask is sound. The
verify-before-comply floor in [bus-trust.md](bus-trust.md) — triggers,
disclosure floor, secrets-never-on-the-bus, and the headless rule — survives
identity enforcement unchanged.
