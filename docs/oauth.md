# Optional OAuth authentication

Static agent and machine tokens continue to work. For Claude, Codex and other
clients that sign users in, ops-brain can act as an OAuth resource server
behind an existing identity provider. It does not issue tokens, host login
pages, register users or run an authorization server.

## Provider requirements

Use an established IdP supporting the MCP authorization flow: authorization
code with S256 PKCE, authorization-server discovery, resource audiences, and
the client's supported registration method (CIMD, DCR or a predefined client).
Configure the exact redirect URL shown by the connecting application. The
IdP must issue RFC 9068 JWT access tokens with:

- `typ` of `at+jwt` or `application/at+jwt`, a `kid`, and RS256 or ES256 signatures.
- The exact configured `iss` and resource `aud`, plus `sub`, `exp`, `iat`,
  `jti` and `client_id`. An optional `nbf` must be numeric.
- Any configured required scopes in the space-delimited `scope` claim.
- A matching explicit subject/client binding in ops-brain.

ID tokens, opaque access tokens, HMAC signatures and arbitrary self-asserted
agent names are rejected. Providers that cannot issue this access-token
format require an appropriate provider configuration or a separately reviewed
adapter. [OpenAI authentication requirements](https://developers.openai.com/plugins/build/auth).

## Configure ops-brain

Set both variables together, using a single line for each in `.env`:

```dotenv
OPS_BRAIN_OAUTH_RESOURCE_URL=https://ops.example.com/mcp
OPS_BRAIN_OAUTH_CONFIG='{"issuer":"https://identity.example.com/","jwks_uri":"https://identity.example.com/.well-known/jwks.json","required_scopes":["ops-brain"],"bindings":[{"subject":"provider-subject-id","oauth_client_id":"registered-client-id","from_agent":"Codex-Buildbox","exclude_client_data":true}]}'
```

The resource URL is canonical, HTTPS, and must identify this deployment.
`audience` can be supplied in the JSON when the provider uses a different
resource identifier; it must still identify this API, not the OAuth client
or another service. The default audience is the resource URL.

`oauth_client_id` is optional in a binding: omitting it allows any validated
client for that subject. Supplying it limits that binding to one client.
Overlapping bindings abort startup. Each binding's optional `client` is
informational, as with static agent tokens; `exclude_client_data` enforces
the existing restricted-seat boundary. Use distinct credentials or principal
bindings for seats that need independent provenance.

The operator bearer remains required; OAuth cannot turn on unauthenticated
development mode. All three Compose files forward these variables. OAuth
credentials reach `/mcp` only, never machine or operator REST endpoints.
Startup logs `external OAuth configured` with its binding count; principal
and OAuth client identifiers are omitted.

## Discovery and validation

GET or HEAD on `/.well-known/oauth-protected-resource/mcp` and the root
`/.well-known/oauth-protected-resource` publishes the resource and its issuer.
Unauthenticated MCP requests receive a `WWW-Authenticate` challenge pointing
at that metadata. The IdP must publish its own authorization-server discovery
metadata. Configure the reverse proxy to forward both well-known paths.

For every request, ops-brain verifies signature, issuer, audience, expiry,
not-before, required scopes and the configured subject/client binding. It
then supplies the same bound identity and visibility used by static agent
tokens. OAuth never grants unbound operator access.

Only the configured HTTPS JWKS endpoint supplies keys. Token-supplied key URLs
are ignored; redirects are disabled. Key fetching has time, size and key-count
bounds, a five-minute cache, and a thirty-second refresh cooldown. Unknown
keys trigger a bounded refresh; stale keys and provider failures fail closed.

Use short-lived access tokens. Self-contained JWT validation cannot observe
instantaneous IdP revocation: removing a server-side binding requires a
restart, while IdP-side revocation may remain unseen until token expiry or
the relevant signing key leaves the cache. There is no introspection or logout webhook.

## Private certificate authorities

Optional `jwks_ca_pem` in the JSON contains a PEM certificate, never a private
key. It adds a trust anchor for the configured JWKS endpoint without disabling
certificate or hostname verification. Public providers do not need it.

## Distinct agent seats

One person can connect Claude and Codex as separate seats using explicit
subject/client bindings. Two hosts using the same subject and OAuth client
share provenance. Use distinct pre-registered clients or provider subjects
when those hosts need independent identities. Model names and tool arguments
cannot create identity boundaries. OAuth bindings use fixed slugs, not families.

## Plugin packages

Build packages for sign-in instead of fixed-token configuration:

```bash
python3 scripts/package-plugin.py --auth oauth \
  --url https://bus.example.com/mcp --output /tmp/ops-brain-oauth-plugins
```

This omits fixed-token headers, token environment variables and Cursor token
inputs. Clients discover the authorization flow from the resource's challenge.
Public listings still need platform review, publisher verification and a
reachable provider. Current OpenAI hosts also check the authorization response
issuer when the provider advertises RFC 9207 support.

For pre-registered public PKCE clients, add `--target claude`, `--target codex` or `--target grok`
and `--oauth-client-id your-public-client-id`. Build one client at a time;
the ID is public, and the builder never accepts a client secret. Register the
host's actual redirect URI with your provider. Codex local packages include
its native `oauth.client_id` setting; Claude uses `oauth.clientId`.
Grok also uses `oauth.clientId`. Add `--oauth-scope ops-brain` for the scope
required by this API; repeat the option when requesting additional scopes.
For providers requiring an exact loopback redirect, add
`--oauth-callback-port 47832` and register
`http://127.0.0.1:47832/callback`. The callback-port and scope options are
Grok-specific. See the [package commands](../plugins/ops-brain-bus/README.md#build-for-your-client).

Use `--target openai --auth oauth` for the separate portable public-submission
artifact. It contains root `plugin.json` and `mcp.json` files following the
[OpenAI package contract](https://developers.openai.com/plugins/build/plugins).
Use `--target codex` for local installation; the
[client format gotcha](../GOTCHAS.md#mcp-clients) explains why the artifacts
remain separate. Public portal validation remains untested.

### OpenAI review cases

[`contrib/openai/review-cases.json`](../contrib/openai/review-cases.json) supplies
five positive and three negative cases, included only in the `openai` package
under `extensions.com.openai.review.test_cases`. These are prepared review
scenarios, not recorded passes in ChatGPT or Codex with a reviewer account.
The existing fixture and transport tests establish related behavior only.

Before running the cases, use a dedicated demo dataset with seats
`Review-Worker` and `Review-Peer`. Bind the reviewer's OAuth identity to
`Review-Worker`; allow it to read its own handoffs and write global knowledge.
Seed a pending action from `Review-Peer` to `Review-Worker` titled
`[Demo] Acknowledge receipt`, whose entire task is to acknowledge receipt.
Also seed a completed action from `Review-Worker` to `Review-Peer` titled
`[Demo] Completed lookup`, with a notify reply titled
`[Demo] Lookup acknowledgement` containing `Sample status confirmed.` and
linked by `in_reply_to`. Reset only this synthetic data before a fresh run.
The peer need not be online: creating a handoff queues work.

Run all eight prompts in the target hosts, record their actual calls and
outcomes, and capture the demo recording before submission. Reviewer access
and sign-in instructions belong in the portal's secure review details, never
in the package or repository. No review account, credentials or seeded data
are created by the package builder. The
[submission contract](https://developers.openai.com/plugins/deploy/submission#configure-onboarding-review-and-publication)
defines the imported fields.

## Validation status

Packaging checks passed Claude Code 2.1.312's strict directory/marketplace
validators and Grok 1.0.70's plugin validator on 2026-10-29.

Unit fixtures exercise verification, key rotation/failure, discovery,
existing static authentication and identity/visibility propagation through
the MCP transport.

An isolated HTTPS provider fixture exercises authorization code with S256 PKCE,
one-time codes, resource audiences, distinct client-bound seats, an actual handoff,
claim, completion and threaded reply. It auto-consents synthetic accounts;
it is test infrastructure and must never front real data.

```bash
cargo build
python3 scripts/test-oauth-flow.py --start-postgres
```

This requires OpenSSL, PostgreSQL 18 with pgvector, and local socket access.
Alternatively set `OPS_BRAIN_TEST_DATABASE_URL` to a disposable database named
`*_test` and omit `--start-postgres`. The fixture never falls back to the
deployment's `DATABASE_URL` or reads its `.env`. It creates completed synthetic
handoffs in the selected test database. Temporary cluster mode removes that
cluster on exit.

On Linux, test native installed plugins without model requests:

```bash
python3 scripts/test-oauth-flow.py --start-postgres --codex --claude --grok
```

This requires `bwrap` and the selected installed CLIs. Each flag can run
independently; `--codex-binary`, `--claude-binary` and `--grok-binary` select standalone
executables when PATH uses launchers. A mount namespace hides the real home
and gives each child a temporary home at its normal path. `HOME` and
`CODEX_HOME` keep their original values. Configuration, credentials and
certificate trust changes stay inside the fixture and are removed on exit.

Verified with Codex 0.165.0, Claude Code 2.1.312 and Grok Build 1.0.70 on 2026-10-29:

- Claude and Codex install the generated plugin from its local marketplace and
  complete native OAuth sign-in against the HTTPS fixture.
- Codex's app-server runtime discovers all 13 tools and runs handoff acceptance,
  completion and a threaded reply, including identity and visibility refusals.
- Claude Code's headless sign-in completes in a temporary terminal, and a
  separate `mcp list` process reconnects with the persisted grant.
- Grok installs the generated plugin by its local path. Its ACP runtime
  completes native OAuth with the configured scope and fixed callback,
  discovers 13 tools, and runs handoff acceptance, completion, a threaded
  reply and identity/visibility refusals. A fresh process reconnects with
  the saved grant. Its synthetic API configuration points at the fixture,
  and no model requests are made.

Real Auth0 verification on 2026-10-29 passed with RFC 9068 access tokens and
installed Codex 0.165.0 and Grok Build 1.0.70 plugins in their regular profiles.
Both discovered 13 tools and reconnected in fresh processes with saved grants,
using separate public clients bound to their own seats. Native Codex exercised
handoff/thread operations; native Grok checked reads and identity refusal.
A model-selected Codex-to-Grok handoff passed retrieval, acceptance, a threaded
reply and completion, and the sender retrieved the reply through its OAuth
plugin. Grok used only the four MCP operations needed for that acknowledgement.

Add `--rust-tests` to run the Rust suite against the same isolated database.

Real-provider Claude Code, Claude web, Cursor sign-in and public directory
approval remain unverified. Operator provider setup remains necessary for
each new seat outside the fixture. No new scheduling or delivery path is added.
