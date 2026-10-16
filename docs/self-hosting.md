# Self-hosting

ops-brain runs for one trusted team per deployment. It remains useful with
full-text search alone; embeddings and external OAuth/events are optional.
The examples are deployment guidance, not a claim of end-to-end validation
on every hosting platform.

## Set up private credentials

From a checkout, choose a published release explicitly:

```bash
python3 scripts/setup-self-host.py --directory /path/to/deployment \
  --image-tag v6.0.0 --agent Claude-Laptop --agent Codex-Buildbox
```

The helper writes `.env` with mode 0600 and an `agent-tokens/` directory with
mode 0700 containing separate 0600 token files. It refuses to overwrite
existing credentials and never prints token values. Keep `.env` on the server
and deliver each seat only its own token file through a trusted channel.

Copy `docker-compose.example.yml` into that deployment directory. Its image
is pinned and its HTTP port binds loopback by default:

```bash
docker compose -f docker-compose.example.yml up -d
curl -fsS http://127.0.0.1:3000/ready
```

The pinned v6.0.0 image predates the OAuth/events/retry additions in this
checkout. Until a release containing those changes is published, build this
checkout with `docker build -t ops-brain:local .` and set
`OPS_BRAIN_IMAGE=ops-brain:local` in the private `.env`. Do not assume `latest`
contains an unreleased change.

## Reach the server from cloud agents

Cloud agents need a reachable HTTPS endpoint. For a reverse proxy on the
same host, keep the default loopback binding and configure Caddy:

```caddyfile
ops.example.com {
    reverse_proxy 127.0.0.1:3000
}
```

Set `OPS_BRAIN_ALLOWED_HOSTS=ops.example.com,ops.example.com:443` in `.env`,
recreate the service, and point clients at `https://ops.example.com/mcp`.
Preserve the Host, Authorization, MCP protocol and session headers. Disable
proxy buffering where it interferes with streaming responses. Restrict the
backend listener to the proxy; TLS must protect bearer credentials.

A containerized proxy has its own loopback. Put it and ops-brain on a shared
Docker network and proxy to `ops-brain:3000` instead; do not publish the
backend publicly merely to make that connection work. For an existing
PostgreSQL/proxy installation, use `docker-compose.prod.yml` with its explicit
networks and always pass `-f docker-compose.prod.yml`.

Connect clients using [clients.md](clients.md). OAuth and event delivery need
the additional setup described in their own guides.

## Backup, upgrade and restore

Back up the database and private deployment configuration separately. For
the standalone Compose file, a restrictive shell umask protects the dump:

```bash
umask 077
docker compose -f docker-compose.example.yml exec -T postgres \
  pg_dump -U ops_brain -d ops_brain -Fc > ops-brain.dump
```

Use encrypted storage and an appropriate retention policy. A database with
events enabled contains callback signing secrets as well as coordination
content. Keep the operator token and agent token files in a separate secret
backup; do not attach them to issues or handoffs.

Before upgrading, take a backup, read the changelog, and select the new
published image tag in `OPS_BRAIN_IMAGE`. Pull that image and recreate the
service with the same explicit Compose file. Startup applies new migrations;
old migration files must never be edited. Check `/ready` and run the
read-only MCP smoke check. MCP sessions are process-local, so clients may
need to reconnect after a restart; run one replica per deployment.

Restore into a fresh database with pgvector available, using the matching
application release and configuration:

```bash
docker compose -f docker-compose.example.yml exec -T postgres \
  pg_restore -U ops_brain -d ops_brain --exit-on-error < ops-brain.dump
```

The target must be empty; restore is not a merge into a live database. Test
restores in isolation. A rollback across a schema change may require restoring
the pre-upgrade backup rather than merely selecting the old image. Old
callback subscriptions restored from backup must not resume unexpectedly:
keep events disabled, remove stale subscriptions, and reconnect consumers
before enabling event delivery.
