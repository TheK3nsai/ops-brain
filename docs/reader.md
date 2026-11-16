# The bus reader

A read-only web view of the handoffs, for the person who runs the deployment.
It answers the question the briefing cannot: *what are my agents actually
saying to each other?* The briefing lists titles and counts. The reader shows
the thread.

It is off by default and costs agents nothing: no MCP tool, no word in the
instructions, no new table.

## Pages

| Page | Shows |
|---|---|
| **Inbox** `/` | What is waiting on you, failing checks, and stuck work: the briefing's sections, live, each one a link to its thread |
| **Open** `/open` | Every open action handoff, grouped by who owes it; self-addressed claims listed apart as locks |
| **Threads** `/threads` | Conversations, most recent activity first |
| **Thread** `/thread/<id>` | One conversation top to bottom, bodies rendered from markdown. Any handoff id in the thread opens it |
| **Timeline** `/timeline` | Everything the bus carried, newest first, by day |
| **Agents** `/agents` | Every slug a handoff has named, with what it filed, received and still owes. Read off the handoffs: there is no registry and no presence |
| **Search** `/search?q=` | The same full-text search as `search_bus` |

Inbox and Open ask the browser to reload every five minutes, so they can stay
open on a screen.

## Turn it on

The reader is a second listener beside the main one. Set its address and, for
anything but a loopback bind, a token:

```bash
cat >> .env <<EOF
OPS_BRAIN_READER_LISTEN=0.0.0.0:3210
OPS_BRAIN_READER_TOKEN=$(openssl rand -hex 32)
OPS_BRAIN_READER_OPERATOR=Operator
OPS_BRAIN_READER_UTC_OFFSET=-04:00
EOF
docker compose -f docker-compose.example.yml up -d
```

| Setting | Meaning |
|---|---|
| `OPS_BRAIN_READER_LISTEN` | Address to serve on. Unset or blank leaves the reader off. In a container use `0.0.0.0:3210`; the compose files publish that port on the host's loopback only |
| `OPS_BRAIN_READER_TOKEN` | The password your browser is asked for (HTTP Basic, any username). At least 32 characters, and not a token the server already uses. Required unless the listen address is loopback |
| `OPS_BRAIN_READER_OPERATOR` | The slug your agents address you by; its open queue is the inbox. Defaults to `Operator`, as the briefing does. The inbox says so when no handoff has ever named the slug |
| `OPS_BRAIN_READER_UTC_OFFSET` | Offset for displayed times, such as `-04:00`. Defaults to UTC. It is a fixed offset, so a zone with daylight saving needs changing twice a year |

Startup fails, rather than warns, when the listen address is not loopback and
no token is set. A container's `0.0.0.0` is reachable by every other container
on its networks, so "it is only published on localhost" is not a reason to run
without one.

Running the binary directly on your own machine, a loopback address needs no
token:

```bash
OPS_BRAIN_READER_LISTEN=127.0.0.1:3210 ops-brain --transport http
```

In that mode the reader answers only requests addressed to `localhost`,
`127.0.0.1` or `[::1]`, which stops another website from reading it through
DNS rebinding. Any local user can still open it.

## Reach it

**Over SSH**, which needs nothing new exposed:

```bash
ssh -NL 3210:127.0.0.1:3210 your-host
# then open http://localhost:3210
```

The SSH server has to permit that forward for your user. A hardened `sshd`
often has forwarding off, and the tunnel then opens but every request fails
with `open failed: administratively prohibited`. The narrow fix is a `Match
User` block with `AllowTcpForwarding local` and `PermitOpen 127.0.0.1:3210`,
which allows this one destination and nothing else.

**Behind your reverse proxy**, when you want it on a phone. Give it its own
hostname; the pages link from the root, so a sub-path will not work. Caddy:

```
reader.example.com {
    reverse_proxy ops-brain:3210
}
```

The token is then the only thing between the internet and every handoff, and
the reader does no rate limiting. Put the hostname behind an identity gate if
you have one: a VPN, or an access proxy at your CDN or in front of your
reverse proxy (Cloudflare Access, an OAuth proxy). Keep the token as well.
The gate decides who may reach the reader at all, so nothing unauthenticated
touches it; the token is the lock the reader checks itself, and it still
holds if the gate is ever removed or scoped to the wrong hostname.

Two things to get right:

- **Gate first, route second.** Create the gate for the hostname before the
  proxy route exists, and check that an anonymous request is turned away by
  the gate rather than by the reader's `401`. In the other order the reader
  is on the internet behind the token alone until you finish.
- **Leave the reader's headers alone.** It sends its own response policy
  (`Content-Security-Policy`, `X-Frame-Options: DENY`, `Referrer-Policy`,
  `Cache-Control: no-store`). A shared security-headers snippet that sets the
  same names either duplicates them or replaces them with looser values. The
  one header that is the proxy's to add is `Strict-Transport-Security`.

## What it will not do

- **Write.** Every route is a GET. The reader has no agent identity, so it
  cannot file, accept, complete or delete. An `action` handoff can wake a
  headless agent session, which makes a browser that can file one a remote
  control for every host on the bus. To answer an agent, tell the agent.
- **Run anything.** Pages contain no script and the response policy
  (`default-src 'none'`) allows none. Handoff bodies are written by agents,
  some of which read mail and web pages, so the reader treats them as
  hostile: raw HTML is shown as text, images are never loaded (an image URL
  is a read receipt), and a link keeps its destination only if it is `http`,
  `https` or `mailto`.
- **Measure.** No charts, no latency, no history of who accepted what when.
  See `ROADMAP.md` on measurement.
- **Narrow what you see.** It is the operator's view of the whole bus.
  Restricted tokens (`exclude_client_data`) do not apply to it; do not hand
  the reader token to anyone who should not read every handoff.

Pages are sent `no-store`, so the browser does not keep handoffs on disk.

## Is it being used?

The server logs one line per page opened:

```
INFO ops_brain::reader: reader view path=/threads
```

That is the whole usage record. If you are deciding whether to leave the
reader on, a month of logs with no such line means nobody reads it.
