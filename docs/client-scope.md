# Client-scope disclosure guard

One deployment is one trusted coordination domain. Per-agent tokens bind provenance, not tenant authorization; every authenticated MCP agent can read fleet-wide handoffs and mutate objects by ID. Client scoping is a knowledge disclosure guard inside that trust domain, not hostile-user isolation. Use separate deployments across real trust boundaries.

1. **Scoped queries withhold by default**: cross-client surfacing requires explicit `acknowledge_cross_client: true` and is audit-logged.
2. **Withhold-by-default on scope mismatch**: when search tools would surface cross-client knowledge, content is **withheld** and replaced with a scope mismatch notice. An explicit `acknowledge_cross_client: true` parameter on a second call releases the result. A gate, not a banner.
3. **Provenance in all results**: every surfaced entry includes `_client_slug` and `_client_name`. Global content (no `client_id`) shows `_client_name: "Global"`.
4. **Audit trail**: `audit_log` table records every cross-client surfacing attempt with tool_name, requesting/owning client_id, entity details, and timestamp.

## Gate behavior

- `client_id IS NULL` → always allowed (global content)
- Same client as requesting → always allowed
- Different client + `cross_client_safe = true` → allowed (audit logged as `released_safe`)
- Different client + `cross_client_safe = false` + `acknowledge_cross_client = true` → released (audit logged)
- Different client + `cross_client_safe = false` + no acknowledgment → **WITHHELD** (notice returned, audit logged)
- No `client_slug` on the query → gate is inert (all rows returned), but every item carries provenance and the response carries a `_note` saying the gate is off (v4.1.0)

## Restricted tokens

Tokens with `exclude_client_data: true` sit outside this gate entirely. The repo queries take a `Visibility` (`src/auth.rs`) and filter in SQL: global knowledge only, and only handoffs the token sent or received. By-id tools return not-found outside that boundary, and such a token can edit only the global rows it wrote. Contract: [agent-tokens.md](agent-tokens.md).
