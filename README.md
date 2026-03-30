# ops-brain

Operational intelligence MCP server for IT infrastructure management. Built for solo IT admins and small MSPs who use Claude Code across multiple machines.

## What It Does

ops-brain is an [MCP](https://modelcontextprotocol.io/) server that models IT infrastructure as a first-class domain — servers, services, sites, clients, networks, vendors, runbooks, incidents, and knowledge — all linked together in a relational database. Instead of re-explaining your infrastructure every session, Claude Code queries ops-brain for instant situational awareness.

**One tool call:**
```
get_situational_awareness(server_slug: "web-server-01")
```
**Returns:** Server details, site, client, all services (with ports), network config, recent incidents with resolutions, relevant runbooks (including semantically related ones), vendor contacts, pending handoffs, knowledge entries, and live monitoring status.

## Key Features

- **74 MCP tools** covering inventory, runbooks, incidents, knowledge, monitoring, ticketing, briefings, and cross-machine coordination
- **Hybrid search** — full-text (tsvector + websearch_to_tsquery) combined with semantic search (pgvector + nomic-embed-text) via Reciprocal Rank Fusion
- **Multi-instance Uptime Kuma** — aggregate monitoring from multiple Kuma instances with partial failure tolerance
- **Proactive monitoring** — background watchdog polls Uptime Kuma, auto-creates/resolves incidents with severity logic, flap suppression, and deduplication
- **Staleness tracking** — tiered alerts for runbooks (30d), knowledge (60d), and services (90d) that haven't been verified
- **Cross-machine coordination** — sessions and handoffs let multiple Claude Code instances collaborate on shared infrastructure
- **Client-scope safety** — default-deny cross-client data gate for multi-client environments with different compliance domains (e.g. HIPAA vs tax/accounting)
- **Zammad integration** — ticket CRUD, search, and bi-directional linking to incidents/servers/services
- **Scheduled briefings** — daily/weekly operational summaries via MCP tool or REST API
- **Fuzzy slug matching** — typo-tolerant lookups with "Did you mean: ...?" suggestions via pg_trgm

## Quick Start

### Docker Compose (recommended for trying it out)

```sh
git clone https://github.com/TheK3nsai/ops-brain.git
cd ops-brain
cp .env.example .env
docker compose up -d
```

This starts PostgreSQL (with pgvector) and ops-brain in HTTP mode. No external services required — monitoring, ticketing, and embeddings are all optional.

Verify it's running:
```sh
curl http://localhost:3000/health   # → OK
```

Then connect Claude Code — add to `~/.claude.json`:
```json
{
  "mcpServers": {
    "ops-brain": {
      "type": "http",
      "url": "http://localhost:3000/mcp"
    }
  }
}
```

Seed with sample data:
```sh
docker compose exec postgres psql -U ops_brain -d ops_brain -f /seed/seed.sql
```

### Local Development

```sh
# Prerequisites: PostgreSQL 16+ with pgvector and pg_trgm extensions
# Optional: ollama with nomic-embed-text for semantic search
# Optional: mold linker for faster builds

git clone https://github.com/TheK3nsai/ops-brain.git
cd ops-brain
cp .env.example .env
# Edit .env with your PostgreSQL connection string

# Option A: Docker for PostgreSQL only
just db-up

# Option B: System PostgreSQL
createuser ops_brain
createdb ops_brain -O ops_brain
psql -U postgres -d ops_brain -c "CREATE EXTENSION IF NOT EXISTS vector;"
psql -U postgres -d ops_brain -c "CREATE EXTENSION IF NOT EXISTS pg_trgm;"

# Build and run (auto-migrates on startup)
cargo run

# Seed with sample data
psql -U ops_brain -d ops_brain -f seed/seed.sql
```

For stdio transport (local Claude Code), add to `~/.claude.json`:
```json
{
  "mcpServers": {
    "ops-brain": {
      "type": "stdio",
      "command": "/path/to/ops-brain/target/release/ops-brain",
      "env": {
        "DATABASE_URL": "postgresql://ops_brain:ops_brain@localhost:5432/ops_brain"
      }
    }
  }
}
```

## Tools (74)

### Inventory (23)
| Tool | Description |
|------|-------------|
| `get_server` | Server details + services, site, networks. Fuzzy slug suggestions on typos ("Did you mean: ...?") |
| `list_servers` | Filter by client, site, role, status. Configurable `limit` (default 50) |
| `get_service` / `list_services` | Service details + which servers run it |
| `get_site` / `get_client` | Entity lookups with related data |
| `get_network` / `get_vendor` | Network and vendor lookups. `get_vendor` accepts `id` (UUID) or `name` |
| `list_vendors` / `list_clients` | List vendors (filter by category/client) and clients |
| `list_sites` / `list_networks` | List sites (filter by client) and networks (filter by site) |
| `search_inventory` | Full-text search across all 10 entity types |
| `upsert_client` / `upsert_site` / `upsert_server` | Create or update records |
| `upsert_service` / `upsert_vendor` | Create or update. `upsert_vendor` accepts `client_slug` to auto-link |
| `upsert_network` | Create or update a network by slug |
| `link_server_service` | Associate a service with a server |
| `delete_server` / `delete_service` / `delete_vendor` | Soft-delete with preview + confirm safety gate |

### Runbooks (7)
| Tool | Description |
|------|-------------|
| `get_runbook` / `list_runbooks` | Retrieve by slug or filter by category/service/server/tag/client |
| `search_runbooks` | Search (mode: fts/semantic/hybrid). Supports `client_slug` scoping + cross-client gate |
| `create_runbook` / `update_runbook` | CRUD with auto-versioning and client ownership |
| `log_runbook_execution` | Record execution — who, result, duration, notes. Compliance audit trail |
| `list_runbook_executions` | Execution history for a runbook or across all |

### Knowledge (3)
| Tool | Description |
|------|-------------|
| `add_knowledge` | Store operational facts, gotchas, tips. Duplicate detection (cosine >85% warns, `force=true` bypasses) |
| `search_knowledge` | Hybrid search across knowledge, runbooks, incidents, handoffs via `tables` param. Browse mode (empty query = recent entries). Cross-client gate |
| `list_knowledge` | Filter by category or client |

### Context (3)
| Tool | Description |
|------|-------------|
| `get_situational_awareness` | **The key tool** — comprehensive briefing for any server, service, or client. `compact=true` (~94K to ~10K), `sections` filtering, `machine` param scopes handoffs |
| `get_client_overview` | Full client briefing with all related data |
| `get_server_context` | Everything about a specific server with cross-client gating |

### Incidents (6)
| Tool | Description |
|------|-------------|
| `create_incident` / `update_incident` | Open/update incidents. Resolving auto-calculates TTR |
| `get_incident` / `list_incidents` | Lookup and filter by client, status, severity |
| `search_incidents` | Search (mode: fts/semantic/hybrid) |
| `link_incident` | Link servers, services, runbooks, and vendors |

### Sessions & Handoffs (10)
| Tool | Description |
|------|-------------|
| `start_session` / `end_session` / `list_sessions` | Work session tracking per machine |
| `create_handoff` / `accept_handoff` / `complete_handoff` | Cross-machine task coordination |
| `delete_handoff` | Permanently delete a handoff by ID (hard delete) |
| `list_handoffs` / `search_handoffs` | Filter/search handoffs. Compact mode (default) truncates bodies |
| `get_catchup` | What changed since a timestamp — handoffs, incidents, knowledge, runbooks, stale runbook warnings |

### Monitoring (7)
| Tool | Description |
|------|-------------|
| `list_monitors` / `get_monitor_status` | Live Uptime Kuma monitor status |
| `get_monitoring_summary` | Quick health: ALL_CLEAR or DEGRADED with down list |
| `link_monitor` / `unlink_monitor` | Map monitors to servers/services. `severity_override` and `flap_threshold` config |
| `list_watchdog_incidents` | Auto-created incidents from proactive monitoring |
| `check_health` | Quick server health based on linked monitors |

### Zammad Ticketing (8)
| Tool | Description |
|------|-------------|
| `list_tickets` / `get_ticket` | List (filter by client/state/priority) or get with full article history |
| `create_ticket` / `update_ticket` | Ticket CRUD, optionally linked to ops-brain incidents |
| `add_ticket_note` | Internal note with optional time accounting |
| `search_tickets` | Full-text search (Elasticsearch syntax) |
| `link_ticket` / `unlink_ticket` | Map Zammad tickets to incidents/servers/services |

### Briefings (3)
| Tool | Description |
|------|-------------|
| `generate_briefing` | Daily or weekly operational summary — monitoring, incidents, handoffs, tickets |
| `list_briefings` / `get_briefing` | Historical briefing access |

### Other (2)
| Tool | Description |
|------|-------------|
| `set_preference` | Global defaults (e.g. `compact=true`). Scoped: global, machine, or client |
| `backfill_embeddings` | Generate embeddings for existing records (batch, with progress) |

## REST API

| Endpoint | Method | Auth | Description |
|----------|--------|------|-------------|
| `/api/briefing` | POST | Bearer token | Generate daily/weekly briefing |
| `/health` | GET | None | Health check |

```sh
curl -s -X POST http://localhost:3000/api/briefing \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"type": "daily"}'
```

## Tech Stack

| Component | Choice |
|-----------|--------|
| Language | Rust |
| MCP SDK | [rmcp](https://github.com/modelcontextprotocol/rust-sdk) 1.2 |
| Database | PostgreSQL 18 |
| SQL | sqlx (async, runtime queries) |
| Async | tokio |
| IDs | UUID v7 (time-ordered) |
| Search | tsvector + GIN (FTS), pgvector HNSW (semantic), pg_trgm (fuzzy) |
| Embeddings | nomic-embed-text (768 dims) via OpenAI-compatible API |
| Monitoring | Uptime Kuma /metrics scraping |
| Ticketing | Zammad REST API |

## Domain Model

```
Client 1──N Site 1──N Server N──N Service
                        │              │
                        N              N
                     Network        Runbook
                                      │
                                      N
Vendor N──────────────────────── Incident
                                   │
Session 1──N Handoff               N
                             Knowledge

Monitor ──── Server (optional)
   └──────── Service (optional)

TicketLink ── Zammad Ticket (external)
   ├──────── Incident (optional)
   ├──────── Server (optional)
   └──────── Service (optional)
```

## Client-Scope Safety

ops-brain is designed for solo operators managing multiple clients with different compliance domains. Since there is no second pair of eyes, the system itself acts as the safety gate to prevent cross-client data leakage.

- **Runbooks**, **knowledge entries**, and **incidents** can be assigned to a client via `client_slug`
- A `cross_client_safe` flag (default: false) controls whether content surfaces outside its owning client
- When cross-client content is detected without acknowledgment, the **actual content is withheld** — only a notice is returned
- Pass `acknowledge_cross_client: true` to release withheld content
- Every access attempt is logged in the `audit_log` table
- All results include `_client_slug` and `_client_name` provenance fields

| Condition | Result |
|-----------|--------|
| `client_id IS NULL` (global) | Always allowed |
| Same client as requesting context | Always allowed |
| Different client + `cross_client_safe = true` | Allowed |
| Different client + `acknowledge_cross_client = true` | Released (audit logged) |
| Different client + no acknowledgment | **Withheld** (audit logged) |

## Environment Variables

| Variable | Default | Description |
|----------|---------|-------------|
| `DATABASE_URL` | (required) | PostgreSQL connection string |
| `OPS_BRAIN_TRANSPORT` | `stdio` | Transport: `stdio` or `http` |
| `OPS_BRAIN_LISTEN` | `0.0.0.0:3000` | HTTP bind address |
| `OPS_BRAIN_AUTH_TOKEN` | (none) | Bearer token for HTTP auth |
| `OPS_BRAIN_MIGRATE` | `true` | Run migrations on startup |
| `OPS_BRAIN_EMBEDDINGS_ENABLED` | `true` | Set `false` to disable embeddings |
| `OPS_BRAIN_EMBEDDING_URL` | `http://localhost:11434/v1/embeddings` | OpenAI-compatible embedding API |
| `OPS_BRAIN_EMBEDDING_MODEL` | `nomic-embed-text` | Embedding model name |
| `OPS_BRAIN_EMBEDDING_API_KEY` | (none) | API key (not needed for ollama) |
| `UPTIME_KUMA_URL` | (none) | Uptime Kuma base URL (single instance, backward compat) |
| `UPTIME_KUMA_USERNAME` | (none) | Basic auth for /metrics (single instance) |
| `UPTIME_KUMA_PASSWORD` | (none) | Basic auth for /metrics (single instance) |
| `UPTIME_KUMA_INSTANCES` | (none) | Multi-instance JSON array (takes precedence). Format: `[{"name":"cloud","url":"..."}]` |
| `OPS_BRAIN_WATCHDOG_ENABLED` | `false` | Enable proactive monitoring |
| `OPS_BRAIN_WATCHDOG_INTERVAL` | `60` | Polling interval (seconds) |
| `OPS_BRAIN_WATCHDOG_CONFIRM_POLLS` | `3` | Consecutive DOWN polls before incident |
| `OPS_BRAIN_WATCHDOG_COOLDOWN_SECS` | `1800` | Cooldown after resolve (seconds) |
| `OPS_BRAIN_WATCHDOG_FLAP_THRESHOLD` | `5` | Auto-downgrade at N recurrences, suppress at 2N |
| `ZAMMAD_URL` | (none) | Zammad API base URL |
| `ZAMMAD_API_TOKEN` | (none) | Zammad API token |
| `ZAMMAD_DEFAULT_OWNER_ID` | (none) | Default ticket assignment |
| `RUST_LOG` | `ops_brain=info` | Tracing filter |

## Planned

- **Web dashboard** — read-only operational view without a Claude session
- **Briefing automation** — server-side cron for daily/weekly email delivery
- **Trend analysis** — daily metric snapshots for backup freshness, disk usage, incident frequency
- **Auto-deploy** — GitHub Actions CD on merge to main

## License

MIT OR Apache-2.0 (dual-licensed)
