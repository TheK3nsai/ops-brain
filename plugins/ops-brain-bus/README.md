# ops-brain-bus plugin

**Handoffs for your AI team.** Connect Claude Code, Codex CLI, Grok Build or
Cursor to your own ops-brain server. Hand work across machines and vendors,
accept it, complete it, and keep the reply attached to the original request.

This plugin contains MCP connection settings and one workflow skill. Run
[ops-brain](https://github.com/TheK3nsai/ops-brain#quick-start) first; the
plugin does not host a server or start another agent. See
[self-hosting](https://github.com/TheK3nsai/ops-brain/blob/main/docs/self-hosting.md)
for deployment and
[client compatibility](https://github.com/TheK3nsai/ops-brain/blob/main/docs/clients.md)
for verification status. A valid package is not proof of an in-app round trip.

## Build for your client

From the repository root, choose your server's public MCP endpoint and a new
output directory:

```bash
python3 scripts/package-plugin.py --url https://bus.example.com/mcp \
  --output /tmp/ops-brain-plugins
```

This builds a folder, local marketplace catalog and ZIP for each client. Add
`--target claude`, `--target codex`, `--target grok` or `--target cursor` to
build just one. Existing output directories are never replaced. The endpoint
is embedded in the generated connection settings; do not publish a package
containing a private hostname. Token values are never read or embedded.

For a server configured with an external OAuth provider, add `--auth oauth`.
The resulting packages discover sign-in through the server and contain no
fixed-token headers or token inputs. See
[OAuth setup](https://github.com/TheK3nsai/ops-brain/blob/main/docs/oauth.md).

If your provider requires a pre-registered public PKCE client, build that
client's package separately:

```bash
python3 scripts/package-plugin.py --url https://bus.example.com/mcp \
  --target codex --auth oauth --oauth-client-id your-public-client-id \
  --output /tmp/ops-brain-codex
```

This option supports Claude, Codex, Grok and the OpenAI submission target and
accepts a public client ID only.
Register each client's actual callback URL with the provider. Use separate
client IDs when the same person needs distinct agent seats. A
confidential client requiring a secret needs the host's secure connection
setup; the builder does not accept client secrets.

For Grok, request the API's scopes explicitly and choose a fixed callback port
when the provider requires an exact redirect URI:

```bash
python3 scripts/package-plugin.py --url https://bus.example.com/mcp \
  --target grok --auth oauth --oauth-client-id your-grok-public-client-id \
  --oauth-callback-port 47832 --oauth-scope ops-brain \
  --output /tmp/ops-brain-grok
```

Register `http://127.0.0.1:47832/callback` for that client. These two options
apply to Grok's pre-registered OAuth client only; repeat `--oauth-scope` for
additional provider-approved scopes. Grok uses a random port when the fixed
port is omitted.

The source `.cursor-plugin/plugin.json` supplies shared metadata, and
`mcp.json`, `skills/` and `assets/` supply the common content. The builder emits
only the manifest fields each target uses, includes both licenses, and uses
an explicit file list. Unrelated files in a checkout are not packaged.

For OpenAI's public submission format, build a separate portable OAuth ZIP:

```bash
python3 scripts/package-plugin.py --url https://bus.example.com/mcp \
  --target openai --auth oauth --output /tmp/ops-brain-openai
```

This emits root `plugin.json` and `mcp.json` files following OpenAI's
[package format](https://developers.openai.com/plugins/build/plugins).
The `all` target builds the four local client packages; it excludes this
submission artifact. Use the `codex` package for local Codex installation;
the [client format gotcha](https://github.com/TheK3nsai/ops-brain/blob/main/GOTCHAS.md#mcp-clients)
explains why the artifacts remain separate. Generating the submission ZIP
does not validate it in OpenAI's portal.

## Connect

Give each agent a credential bound to its own stable slug; keep that slug in
its local instructions. With OAuth, the operator maps the provider subject
and client to that slug, and you complete the client's sign-in flow. With
fixed tokens, use your existing secret manager to make
`OPS_BRAIN_AGENT_TOKEN` available to the client process before launching it.
Use the per-agent credential, never the operator's `OPS_BRAIN_AUTH_TOKEN`.
See [agent tokens](https://github.com/TheK3nsai/ops-brain/blob/main/docs/agent-tokens.md).

Install from the generated local marketplace for the client you use:

```bash
claude plugin marketplace add /tmp/ops-brain-plugins/claude
claude plugin install ops-brain-bus@ops-brain-local
```

```bash
codex plugin marketplace add /tmp/ops-brain-plugins/codex
codex plugin add ops-brain-bus@ops-brain-local
```

```bash
grok plugin install /tmp/ops-brain-plugins/grok/ops-brain-bus
```

Grok 1.0.70 identifies a local marketplace by the directory's basename rather
than the catalog's declared name. Installing the generated plugin path avoids
that naming dependency.

Restart the client after installation. For Cursor, add the generated `cursor/`
marketplace through its plugin settings. Configure the agent token there in
fixed-token mode; OAuth packages omit that input.
The original source plugin also supports configuring `OPS_BRAIN_URL` in
Cursor; generated packages already contain the endpoint passed to the builder.

In fixed-token mode, Claude and Grok use the client's environment expansion
for the Authorization header. Codex uses `bearer_token_env_var` and reads the
credential from the environment. Keep one connection to the bus per client:
when adopting the plugin, remove the equivalent manual MCP entry through that
client's settings to avoid duplicate tools.

These packages do not establish public directory approval. Authenticated
public submissions for Claude and OpenAI need OAuth; configure a reachable
provider before submitting the OAuth variant. Grok Bot and web chat
connections have separate setup and review paths. No public listing or
unattended wake behavior is enabled by building or installing a package.

## Verify with your own work

Start with a read-only connection check:

```bash
python3 scripts/mcp-smoke.py --url https://bus.example.com/mcp \
  --agent Example-Laptop --token-file /absolute/private/path/agent-token
```

Then ask the installed client to "check my pending ops-brain handoffs". For
a real round trip, have one connected agent send a harmless handoff to a
second agent. Ask the recipient to retrieve and accept it, complete it, and
reply in thread. Ask the sender to retrieve that reply. This checks the
actual client's tool selection and identity handling, which a package
validator or HTTP smoke check cannot prove.

The receiving agent must check for work or have a separately configured wake
mechanism. The plugin does not guarantee that a peer is running or schedule
its execution. There is no mandatory session-start check-in.

## Privacy and data handling

This plugin sends MCP requests to the ops-brain endpoint you selected. The
package contains no telemetry, hooks or executable client scripts. The
software authors do not receive bus content through the plugin.

With OAuth, your identity provider handles sign-in and consent and issues
the credential. ops-brain verifies it against that provider's public signing
keys and the operator's subject/client bindings. Access tokens are not retained
in the MCP tool context. Provider data practices and retention are separate
from the self-hosted bus.

Your self-hosted server stores handoffs, shared knowledge and audit records
in its PostgreSQL database. Queued work survives sessions and restarts. FYI
notifications leave operational queries after seven days; that is not data
deletion. Explicit handoff deletion retains an audit tombstone containing
the handoff ID, deleting agent, sender, recipient, title and status. The
operator controls database access, backups and retention.

Content returned to a connected agent is available to that agent's provider
under the provider's terms. If the operator enables a remote embedding
endpoint, knowledge and handoff text on writes, and semantic or hybrid search
queries, also reach that endpoint. Disable embeddings or use a local endpoint
when that content must stay within your deployment.

One deployment is one trusted team. Agent tokens bind provenance; they do not
isolate unrelated customers. Do not put secrets in handoffs or knowledge.
For access, deletion and retention requests, contact your deployment operator.
For software security concerns, use the
[security reporting channel](https://github.com/TheK3nsai/ops-brain/blob/main/SECURITY.md).
