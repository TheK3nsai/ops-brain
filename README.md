# ops-brain

Shared operational intelligence MCP server for IT infrastructure management. Built for solo IT admins and small MSPs who use Claude Code across multiple machines.

## What It Does

ops-brain is an [MCP](https://modelcontextprotocol.io/) server that models IT infrastructure as a first-class domain — servers, services, sites, clients, networks, vendors, runbooks, incidents, and knowledge — all linked together in a relational database. Instead of re-explaining your infrastructure every session, Claude Code queries ops-brain for instant situational awareness.

**One tool call:**
```
get_situational_awareness(server_slug: "hvfs0")
```
**Returns:** Server details, site, client, all services (with ports), network config, recent incidents with resolutions, relevant runbooks (including semantically related ones), vendor contacts, pending handoffs, knowledge entries, and live monitoring status.

## Tools (47)

### Inventory (15)
| Tool | Description |
|------|-------------|
| `get_server` | Server details + services, site, networks |
| `list_servers` | Filter by client, site, role, status |
| `get_service` / `list_services` | Service details + which servers run it |
| `get_site` / `get_client` | Entity lookups with related data |
| `get_network` / `get_vendor` | Network and vendor lookups |
| `search_inventory` | Full-text search across all entities (servers, services, runbooks, knowledge, incidents, handoffs) |
| `upsert_client` / `upsert_site` / `upsert_server` | Create or update records |
| `upsert_service` / `upsert_vendor` | Create or update records |
| `link_server_service` | Associate a service with a server |

### Runbooks (5)
| Tool | Description |
|------|-------------|
| `get_runbook` / `list_runbooks` | Retrieve by slug or filter by category/service/server/tag |
| `search_runbooks` | Search runbook content (mode: fts/semantic/hybrid) |
| `create_runbook` / `update_runbook` | CRUD with auto-versioning |

### Knowledge (3)
| Tool | Description |
|------|-------------|
| `add_knowledge` | Store operational facts, gotchas, tips |
| `search_knowledge` | Search knowledge base (mode: fts/semantic/hybrid) |
| `list_knowledge` | Filter by category or client |

### Context (3)
| Tool | Description |
|------|-------------|
| `get_situational_awareness` | **The key tool** — comprehensive briefing for any server, service, or client (includes live monitoring + semantically related content) |
| `get_client_overview` | Full client briefing with all related data |
| `get_server_context` | Everything about a specific server (includes live monitoring + semantically related runbooks/knowledge) |

### Incidents (6)
| Tool | Description |
|------|-------------|
| `create_incident` | Open a new incident, optionally linking servers and services |
| `update_incident` | Update fields; setting status to `resolved` auto-calculates TTR |
| `get_incident` | Full incident details with linked servers, services |
| `list_incidents` | Filter by client, status, severity |
| `search_incidents` | Search incidents (mode: fts/semantic/hybrid) |
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
| `search_handoffs` | Search handoffs (mode: fts/semantic/hybrid) |

### Monitoring (5)
| Tool | Description |
|------|-------------|
| `list_monitors` | All Uptime Kuma monitors with live status, filterable by up/down/pending/maintenance |
| `get_monitor_status` | Detailed live status for a specific monitor with linked server/service info |
| `get_monitoring_summary` | Quick health check — ALL_CLEAR or DEGRADED with down monitor list |
| `link_monitor` | Map an Uptime Kuma monitor name to an ops-brain server and/or service |
| `unlink_monitor` | Remove a monitor-to-entity mapping |

### Semantic Search (2)
| Tool | Description |
|------|-------------|
| `semantic_search` | AI-powered cross-table search — finds conceptually related content even without exact keyword matches |
| `backfill_embeddings` | Generate embeddings for existing records (batch, with progress reporting) |

## Tech Stack

| Component | Choice |
|-----------|--------|
| Language | Rust |
| MCP SDK | [rmcp](https://github.com/modelcontextprotocol/rust-sdk) 1.2 |
| Database | PostgreSQL 18 |
| SQL | sqlx (async, runtime queries) |
| Async | tokio |
| IDs | UUID v7 (time-ordered) |
| Search | PostgreSQL tsvector + GIN indexes (FTS), pgvector HNSW (semantic) |
| Embeddings | ollama nomic-embed-text (768 dims) via OpenAI-compatible API |
| Monitoring | Uptime Kuma /metrics (Prometheus format, on-demand scraping) |
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
# pgvector extension (requires superuser):
psql -U postgres -d ops_brain -c "CREATE EXTENSION IF NOT EXISTS vector;"

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
      "type": "streamable-http",
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

# For semantic search, run an ollama container on the same Docker network:
# docker run -d --name ollama --network traefik-net ollama/ollama
# docker exec ollama ollama pull nomic-embed-text
# Then set in docker-compose.prod.yml:
# OPS_BRAIN_EMBEDDING_URL=http://ollama:11434/v1/embeddings

# PostgreSQL must use pgvector-enabled image (pgvector/pgvector:pg18)
# and the extension must be created: CREATE EXTENSION IF NOT EXISTS vector;

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
```

## Roadmap

### Completed

- [x] **Phase 1**: Local MCP server — inventory, runbooks, knowledge, context tools (26 tools)
- [x] **Phase 2**: Remote deployment to cloud server (Streamable HTTP + bearer auth)
- [x] **Phase 3**: Incident lifecycle + cross-machine coordination (sessions, handoffs) — 40 tools
- [x] **Phase 4**: Monitoring integration — live Uptime Kuma /metrics scraping, monitor-to-entity mapping — 45 tools
- [x] **Phase 5**: Semantic search — pgvector + ollama embeddings, hybrid RRF ranking, context enrichment — 47 tools

### Planned

- [ ] **Phase 6**: Proactive monitoring — scheduled watchdog that detects Uptime Kuma status changes, auto-creates incidents with linked servers/services/runbooks, and sends notifications
- [ ] **Phase 7**: Zammad integration — link support tickets to ops-brain entities (servers, services, incidents), enrich tickets with situational awareness
- [ ] **Phase 8**: Scheduled briefings — daily/weekly operational summaries via Claude Code scheduled triggers or email

## License

Private. Open-source release planned when polished.
