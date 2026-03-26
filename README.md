# ops-brain

Shared operational intelligence MCP server for IT infrastructure management. Built for solo IT admins and small MSPs who use Claude Code across multiple machines.

## What It Does

ops-brain is an [MCP](https://modelcontextprotocol.io/) server that models IT infrastructure as a first-class domain — servers, services, sites, clients, networks, vendors, runbooks, incidents, and knowledge — all linked together in a relational database. Instead of re-explaining your infrastructure every session, Claude Code queries ops-brain for instant situational awareness.

**One tool call:**
```
get_situational_awareness(server_slug: "hvfs0")
```
**Returns:** Server details, site, client, all services (with ports), network config, recent incidents with resolutions, relevant runbooks (including semantically related ones), vendor contacts, pending handoffs, knowledge entries, and live monitoring status.

## Tools (64)

### Inventory (18)
| Tool | Description |
|------|-------------|
| `get_server` | Server details + services, site, networks. Fuzzy slug suggestions on typos ("Did you mean: ...?") |
| `list_servers` | Filter by client, site, role, status. Configurable `limit` (default 50) |
| `get_service` / `list_services` | Service details + which servers run it. Configurable `limit` |
| `get_site` / `get_client` | Entity lookups with related data |
| `get_network` / `get_vendor` | Network and vendor lookups |
| `search_inventory` | Full-text search across all entities. Configurable `limit` per type (default 10) |
| `upsert_client` / `upsert_site` / `upsert_server` | Create or update records |
| `upsert_service` / `upsert_vendor` | Create or update records |
| `link_server_service` | Associate a service with a server |
| `delete_server` | Soft-delete server by slug with preview + confirm safety gate |
| `delete_service` | Soft-delete service by slug with preview + confirm safety gate |
| `delete_vendor` | Soft-delete vendor by name with preview + confirm safety gate |

### Runbooks (5)
| Tool | Description |
|------|-------------|
| `get_runbook` / `list_runbooks` | Retrieve by slug or filter by category/service/server/tag/client. Configurable `limit` |
| `search_runbooks` | Search runbook content (mode: fts/semantic/hybrid). Configurable `limit`. Supports `client_slug` scoping + `acknowledge_cross_client` gate |
| `create_runbook` / `update_runbook` | CRUD with auto-versioning. Supports `client_slug` ownership + `cross_client_safe` flag |

### Knowledge (3)
| Tool | Description |
|------|-------------|
| `add_knowledge` | Store operational facts, gotchas, tips. Supports `cross_client_safe` flag |
| `search_knowledge` | Search knowledge base (mode: fts/semantic/hybrid). Configurable `limit`. Supports `client_slug` scoping + `acknowledge_cross_client` gate |
| `list_knowledge` | Filter by category or client. Configurable `limit` |

### Context (3)
| Tool | Description |
|------|-------------|
| `get_situational_awareness` | **The key tool** — comprehensive briefing for any server, service, or client. Cross-client auto-gated. `compact=true` (~94K→~10K), `sections` filtering. Returns `_warnings` on transient failures |
| `get_client_overview` | Full client briefing with all related data. Returns `_warnings` on transient failures |
| `get_server_context` | Everything about a specific server. Cross-client auto-gated. `compact=true`, `sections` filtering. Returns `_warnings` on transient failures |

### Incidents (6)
| Tool | Description |
|------|-------------|
| `create_incident` | Open a new incident, optionally linking servers and services. Supports `cross_client_safe` flag |
| `update_incident` | Update fields; setting status to `resolved` auto-calculates TTR. Supports `cross_client_safe` flag |
| `get_incident` | Full incident details with linked servers, services |
| `list_incidents` | Filter by client, status, severity |
| `search_incidents` | Search incidents (mode: fts/semantic/hybrid). Configurable `limit` |
| `link_incident` | Link servers, services, runbooks (with usage tracking), and vendors |

### Sessions (3)
| Tool | Description |
|------|-------------|
| `start_session` | Begin a work session on a machine |
| `end_session` | End a session with an optional summary |
| `list_sessions` | List sessions, filter by machine or active status |

### Handoffs (5)
| Tool | Description |
|------|-------------|
| `create_handoff` | Create a task for another machine/session to pick up |
| `accept_handoff` | Accept a pending handoff |
| `complete_handoff` | Mark a handoff as done |
| `list_handoffs` | Filter by status, source/target machine |
| `search_handoffs` | Search handoffs (mode: fts/semantic/hybrid). Configurable `limit` |

### Monitoring (6)
| Tool | Description |
|------|-------------|
| `list_monitors` | All Uptime Kuma monitors with live status, filterable by up/down/pending/maintenance |
| `get_monitor_status` | Detailed live status for a specific monitor with linked server/service info |
| `get_monitoring_summary` | Quick health check — ALL_CLEAR or DEGRADED with down monitor list |
| `link_monitor` | Map an Uptime Kuma monitor name to an ops-brain server and/or service |
| `unlink_monitor` | Remove a monitor-to-entity mapping |
| `list_watchdog_incidents` | List incidents auto-created by the proactive monitoring watchdog, filterable by status |

### Zammad Ticketing (8)
| Tool | Description |
|------|-------------|
| `list_tickets` | List Zammad tickets filtered by client, state, priority |
| `get_ticket` | Get ticket by ID with full article history |
| `create_ticket` | Create a ticket in Zammad, optionally link to ops-brain incident |
| `update_ticket` | Update ticket state, priority, or title |
| `add_ticket_note` | Add internal note with optional time accounting |
| `search_tickets` | Full-text search across Zammad tickets (Elasticsearch syntax) |
| `link_ticket` | Map a Zammad ticket to ops-brain incident/server/service |
| `unlink_ticket` | Remove a ticket mapping |

### Briefings (3)
| Tool | Description |
|------|-------------|
| `generate_briefing` | Generate a daily or weekly operational briefing — aggregates monitoring, incidents, handoffs, tickets into a structured markdown summary |
| `list_briefings` | List previously generated briefings, filterable by type and client |
| `get_briefing` | Retrieve a specific briefing by ID |

### Semantic Search (2)
| Tool | Description |
|------|-------------|
| `semantic_search` | AI-powered cross-table search — finds conceptually related content. Supports `client_slug` scoping + `acknowledge_cross_client` gate for runbooks/knowledge/incidents |
| `backfill_embeddings` | Generate embeddings for existing records (batch, with progress reporting) |

## REST API

In addition to MCP tools, ops-brain exposes a REST API for external consumers (scheduled triggers, cron jobs, webhooks):

| Endpoint | Method | Description |
|----------|--------|-------------|
| `/api/briefing` | POST | Generate a daily or weekly operational briefing |
| `/health` | GET | Health check (unauthenticated) |

```sh
# Example: generate a daily briefing
curl -s -X POST https://ops.kensai.cloud/api/briefing \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"type": "daily"}'

# Scoped to a specific client
curl -s -X POST https://ops.kensai.cloud/api/briefing \
  -H "Authorization: Bearer <token>" \
  -H "Content-Type: application/json" \
  -d '{"type": "weekly", "client_slug": "hsr"}'
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
| Search | PostgreSQL tsvector + GIN indexes (FTS), pgvector HNSW (semantic), pg_trgm (fuzzy slug matching) |
| Embeddings | ollama nomic-embed-text (768 dims) via OpenAI-compatible API |
| Monitoring | Uptime Kuma /metrics (Prometheus format, on-demand scraping) |
| Ticketing | Zammad REST API (Token auth, live queries) |
| HTTP Client | reqwest (rustls-tls, json) |

## Setup

### Prerequisites

- Rust 1.83+
- PostgreSQL 16+ (18 recommended) with [pgvector](https://github.com/pgvector/pgvector) extension
- [ollama](https://ollama.com/) with `nomic-embed-text` model (for semantic search)
- [just](https://github.com/casey/just) (optional, for dev commands)

### Local Development

```sh
# Clone
git clone https://github.com/TheK3nsai/ops-brain.git
cd ops-brain

# Set up database
cp .env.example .env
# Edit .env with your PostgreSQL connection string

# Option A: Use Docker for PostgreSQL
just db-up

# Option B: Use system PostgreSQL
createuser ops_brain
createdb ops_brain -O ops_brain
# Extensions (require superuser):
psql -U postgres -d ops_brain -c "CREATE EXTENSION IF NOT EXISTS vector;"
psql -U postgres -d ops_brain -c "CREATE EXTENSION IF NOT EXISTS pg_trgm;"

# Pull embedding model
ollama pull nomic-embed-text

# Build and run (auto-migrates on startup)
cargo run

# Seed with sample data
psql -U ops_brain -d ops_brain -f seed/seed.sql
```

### Claude Code Configuration

**Local (stdio)** — add to `~/.claude.json`:

```json
{
  "mcpServers": {
    "ops-brain": {
      "type": "stdio",
      "command": "/path/to/ops-brain/target/release/ops-brain",
      "args": [],
      "env": {
        "DATABASE_URL": "postgresql://ops_brain:ops_brain@localhost:5432/ops_brain"
      }
    }
  }
}
```

**Remote (HTTP)** — deploy with Docker, then configure Claude Code with the URL and auth token:

```json
{
  "mcpServers": {
    "ops-brain": {
      "type": "http",
      "url": "https://ops.example.com/mcp",
      "headers": {
        "Authorization": "Bearer <your-token>"
      }
    }
  }
}
```

### Docker Deployment

```sh
# Build and start (uses shared PostgreSQL)
docker compose -f docker-compose.prod.yml up -d --build

# Environment variables needed in .env:
# OPS_BRAIN_DB_PASSWORD=<postgres password>
# OPS_BRAIN_AUTH_TOKEN=<bearer token for HTTP auth>
# UPTIME_KUMA_URL=http://uptime-kuma:3001  (optional, for monitoring integration)
# UPTIME_KUMA_USERNAME=admin               (if /metrics requires basic auth)
# UPTIME_KUMA_PASSWORD=<password>          (if /metrics requires basic auth)
# OPS_BRAIN_WATCHDOG_ENABLED=true          (enable proactive monitoring watchdog)
# OPS_BRAIN_WATCHDOG_INTERVAL=60           (polling interval in seconds)
# ZAMMAD_URL=http://zammad-railsserver:3000  (optional, for ticketing integration)
# ZAMMAD_API_TOKEN=<token>                 (Zammad API token)

# For semantic search, run an ollama container on the same Docker network:
# docker run -d --name ollama --network traefik-net ollama/ollama
# docker exec ollama ollama pull nomic-embed-text
# Then set in docker-compose.prod.yml:
# OPS_BRAIN_EMBEDDING_URL=http://ollama:11434/v1/embeddings

# PostgreSQL must use pgvector-enabled image (pgvector/pgvector:pg18)
# Extensions auto-created by migrations: vector, pg_trgm

# Seed the database
cat seed/seed.sql | docker exec -i shared-postgres psql -U ops_brain -d ops_brain
```

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

## Client-Scope Safety (Phase 9)

ops-brain serves a solo operator managing two clients with different compliance domains (HIPAA hospice vs IRS/tax CPA). Since there is no second pair of eyes, the system itself acts as the safety gate to prevent cross-client data leakage.

### How It Works

- **Runbooks**, **knowledge entries**, and **incidents** can be assigned to a client via `client_slug` (unset = global)
- A `cross_client_safe` flag (default: false) controls whether content can surface outside its owning client
- **Context tools** (`get_situational_awareness`, `get_server_context`) automatically resolve the client from the server/service chain and gate runbooks/knowledge/incidents
- **Search tools** accept optional `client_slug` to explicitly scope results
- When cross-client content is detected without acknowledgment, the **actual content is withheld** — only a notice with count and owning client is returned
- Pass `acknowledge_cross_client: true` on a second call to release withheld content
- Every cross-client access attempt is logged in the `audit_log` table
- All surfaced items include `_client_slug` and `_client_name` provenance fields
- The watchdog only suggests same-client or global runbooks for auto-created incidents

### Gate Rules

| Condition | Result |
|-----------|--------|
| `client_id IS NULL` (global) | Always allowed |
| Same client as requesting context | Always allowed |
| Different client + `cross_client_safe = true` | Allowed |
| Different client + `acknowledge_cross_client = true` | Released (audit logged) |
| Different client + no acknowledgment | **Withheld** (audit logged) |

## Roadmap

### Completed

- [x] **Phase 1**: Local MCP server — inventory, runbooks, knowledge, context tools (26 tools)
- [x] **Phase 2**: Remote deployment to cloud server (Streamable HTTP + bearer auth)
- [x] **Phase 3**: Incident lifecycle + cross-machine coordination (sessions, handoffs) — 40 tools
- [x] **Phase 4**: Monitoring integration — live Uptime Kuma /metrics scraping, monitor-to-entity mapping — 45 tools
- [x] **Phase 5**: Semantic search — pgvector + ollama embeddings, hybrid RRF ranking, context enrichment — 47 tools
- [x] **Phase 6**: Proactive monitoring — background watchdog polls Uptime Kuma, detects UP/DOWN transitions, auto-creates/resolves incidents with TTR, links servers/services/runbooks via semantic search, input validation — 48 tools
- [x] **Phase 7**: Zammad integration — live Zammad REST API queries, ticket CRUD with time accounting, ticket-to-entity linking, context tools enriched with ticket data — 56 tools
- [x] **Phase 8**: Scheduled briefings — daily/weekly operational summaries aggregating monitoring, incidents, handoffs, and tickets with historical storage, REST API, Gmail delivery via scheduled triggers — 59 tools (before Phase 9 additions)

- [x] **Phase 9**: Client-scope safety — default-deny cross-client content surfacing (`cross_client_safe` flag on runbooks/knowledge/incidents), withhold-by-default gate pattern (`acknowledge_cross_client` parameter), provenance attribution (`_client_slug`/`_client_name` in results), audit trail (`audit_log` table), watchdog client-scoped runbook suggestions, `compact` mode + `sections` filtering for context tools — 64 tools

**Post-phase improvements:**

- [x] Fuzzy slug suggestions (pg_trgm "Did you mean: ...?" on 404s)
- [x] UNION incident queries (single query replaces N+1 pattern)
- [x] Push monitor diagnostic hints (DOWN = heartbeat expired, not service failure)
- [x] `_warnings` array on context tools (surfaces transient sub-query failures instead of silent empty results)
- [x] Consistent result limits (`limit` param on all list/search tools, standardized defaults)
- [x] Soft deletes (servers/services/vendors set `status='deleted'` — FK references and audit trail preserved)

## License

Private. Open-source release planned when polished.
