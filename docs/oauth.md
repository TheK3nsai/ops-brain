# Optional OAuth authentication

Static agent and machine tokens continue to work. For ChatGPT Dots and other
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
OPS_BRAIN_OAUTH_CONFIG='{"issuer":"https://identity.example.com/","jwks_uri":"https://identity.example.com/.well-known/jwks.json","required_scopes":["ops-brain"],"bindings":[{"subject":"provider-subject-id","oauth_client_id":"registered-client-id","from_agent":"Dot-Workspace","exclude_client_data":true}]}'
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
the relevant signing key leaves the cache. Event subscriptions expire no
later than their authenticated access token and recheck the configured
binding before delivery. There is no introspection or logout webhook.

## Validation status

Local fixtures exercise verification, key rotation/failure, discovery,
existing static authentication and identity/visibility propagation through
the MCP transport. No live provider, ChatGPT account-linking flow or Dot was
used to validate this integration. Operator provider setup remains necessary
before enabling it.
