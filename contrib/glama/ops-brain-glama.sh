#!/bin/sh
# Glama check wrapper: local throwaway Postgres + pgvector, then ops-brain on stdio.
# Stdout carries MCP JSON-RPC only; everything else goes to stderr.
set -eu
PG_BIN="${PG_BIN:-/usr/lib/postgresql/17/bin}"
PGDATA="${PGDATA:-/var/lib/ops-brain-glama/pg}"
SOCK="${PGSOCK:-/run/postgresql}"
as_pg() { if [ "$(id -u)" = 0 ]; then runuser -u postgres -- "$@"; else "$@"; fi; }
mkdir -p "$PGDATA" "$SOCK"
[ "$(id -u)" = 0 ] && chown -R postgres:postgres "$PGDATA" "$SOCK"
if [ ! -s "$PGDATA/PG_VERSION" ]; then
  as_pg "$PG_BIN/initdb" -D "$PGDATA" -U postgres -A trust >&2
fi
as_pg "$PG_BIN/pg_ctl" -D "$PGDATA" status >/dev/null 2>&1 ||
  as_pg "$PG_BIN/pg_ctl" -D "$PGDATA" -l "$PGDATA/server.log" -w \
    -o "-c listen_addresses=127.0.0.1 -p 5432 -k $SOCK" start >&2
as_pg "$PG_BIN/psql" -h 127.0.0.1 -U postgres -d postgres -qtAc \
  "SELECT 1 FROM pg_database WHERE datname = 'ops_brain'" | grep -q 1 ||
  as_pg "$PG_BIN/createdb" -h 127.0.0.1 -U postgres ops_brain >&2
as_pg "$PG_BIN/psql" -h 127.0.0.1 -U postgres -d ops_brain -qc \
  "CREATE EXTENSION IF NOT EXISTS vector" >&2
# Throwaway values only. Placeholders Glama injects are overridden here.
export DATABASE_URL="postgres://postgres@127.0.0.1:5432/ops_brain"
export OPS_BRAIN_TRANSPORT=stdio
export OPS_BRAIN_EMBEDDINGS_ENABLED=false
export OPS_BRAIN_MIGRATE=true
unset OPS_BRAIN_AGENT_TOKENS OPS_BRAIN_MACHINE_TOKENS OPS_BRAIN_AUTH_TOKEN
exec "${OPS_BRAIN_BIN:-/usr/local/bin/ops-brain}"
