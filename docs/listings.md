# Listings

Where ops-brain is published, and how each listing stays current.

| Where | State | How it updates |
|---|---|---|
| [Official MCP Registry](https://registry.modelcontextprotocol.io/) as `io.github.TheK3nsai/ops-brain` | live | The release workflow publishes `server.json` through GitHub OIDC after the image push, on `v*` tags only. |
| [Glama](https://glama.ai/mcp/servers/TheK3nsai/ops-brain) | claimed; license A; auto-release on | Glama re-syncs on push; after a `v*` tag, Sync Server (below). Its check config is the admin form, snapshot in [`contrib/glama/`](../contrib/glama/). |
| [HOL Guard](https://github.com/hashgraph-online/hol-guard/blob/main/contributions/mcp-servers/mcp.ops-brain.json) catalog as `mcp.ops-brain` | listed (stdio entry, opt-in); `hashgraph-online/hol-guard#5255` | No version pin; bump the entry only if tool names change. |
| [mcp.so](https://mcp.so/) | requested | Submission pending in `chatmcp/mcpso#1`. |
| GitHub MCP Registry | requested | Pending with GitHub. |
| Grok, Grok Bot and Cursor marketplaces (`ops-brain-bus` plugin) | submitted 2026-10-17 via the x.ai form | [`plugins/ops-brain-bus/`](../plugins/ops-brain-bus/), listed by the root `.cursor-plugin/marketplace.json`. Updates are re-reviewed. |
| Grok Build catalog (`ops-brain-bus`) | requested | `xai-org/plugin-marketplace#1520`, pinned to a commit SHA; a plugin change needs a new pin. |

## Glama check

Glama builds its own image from a form (base image, build steps, CMD, env schema, placeholders), wraps the CMD with `mcp-proxy`, and checks `initialize` + `tools/list` over stdio.

[`contrib/glama/form.json`](../contrib/glama/form.json) holds the values to paste:

1. Install PostgreSQL 17 and pgvector from Debian trixie main, plus `curl` and `jq`.
2. Pull the `ghcr.io/thek3nsai/ops-brain:latest` linux/amd64 binary: resolve the manifest, verify the manifest and layer digests, and take `app/ops-brain` from the topmost layer that has it. A missing binary fails the build.
3. Install [`ops-brain-glama.sh`](../contrib/glama/ops-brain-glama.sh) from the clone at `/app`. It starts a throwaway trust-auth Postgres on loopback, creates the `vector` extension, unsets every token variable, and execs ops-brain on stdio with embeddings off.

The placeholders are dummies the script overrides. The check runs the latest *released* binary, not the synced commit, so between a merge and the next tag it tests the previous release.

## On each release

After the `v*` tag's release workflow is green (the MCP Registry publishes itself), click **Repository → Sync Server** on Glama's admin page. Auto-release is on, so Glama builds and publishes by itself and emails the owner "Build succeeded" and "Release X published".

Verify with those emails or the listing's release number. The Tool Schema Changelog can lag and lists schema changes only.
