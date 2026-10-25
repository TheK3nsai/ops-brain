# Listings

Where ops-brain is published, and how each listing stays current.

| Where | State | How it updates |
|---|---|---|
| [Official MCP Registry](https://registry.modelcontextprotocol.io/) as `io.github.TheK3nsai/ops-brain` | live | The release workflow publishes `server.json` through GitHub OIDC after the image push, on `v*` tags only. |
| [Glama](https://glama.ai/mcp/servers/TheK3nsai/ops-brain) | claimed; license A; auto-release on | Glama re-syncs on push; after a `v*` tag, Sync Server (below). Its check config is the admin form, snapshot in [`contrib/glama/`](../contrib/glama/). |
| [HOL Guard](https://github.com/hashgraph-online/hol-guard/blob/main/contributions/mcp-servers/mcp.ops-brain.json) catalog as `mcp.ops-brain` | listed (stdio entry, opt-in); `hashgraph-online/hol-guard#5255`. [Extension page](https://hol.org/guard/extensions/mcp.ops-brain) claimed via GitHub, setup guide published, launch story in moderation | No version pin; bump the entry only if tool names change. |
| [mcprush](https://mcprush.com/thek3nsai/ops-brain-mcp) as Ops Brain MCP | listed by mcprush on its own 2026-10-19; claim submitted 2026-10-19 via GitHub, pending their review | They index the `thek3nsai/ops-brain` container image. Once claimed, bump the version by hand after each release. |
| [Docker MCP Catalog](https://hub.docker.com/mcp) as `mcp/ops-brain` | submitted 2026-10-19; [`docker/mcp-registry#5871`](https://github.com/docker/mcp-registry/pull/5871), CI pending maintainer approval (first-time contributor) | Docker builds the image from a pinned commit in `servers/ops-brain/server.yaml`; updates come from Docker's nightly pin-bump PRs against `main`. |
| [mcp.so](https://mcp.so/) | requested | Submission pending in `chatmcp/mcpso#1`. |
| [GitHub MCP Registry](https://github.com/mcp) as `io.github.TheK3nsai/ops-brain` | accepted 2026-10-20 | Draws from the Official MCP Registry entry ([`v0/servers?search=ops-brain`](https://registry.modelcontextprotocol.io/v0/servers?search=ops-brain)); 6.3.2 is latest there, so no separate publish step. |
| Grok, Grok Bot and Cursor marketplaces (`ops-brain-bus` plugin) | submitted 2026-10-17 via the x.ai form | [`plugins/ops-brain-bus/`](../plugins/ops-brain-bus/), listed by the root `.cursor-plugin/marketplace.json`. Updates are re-reviewed. |
| Grok Build catalog (`ops-brain-bus`) | requested; pin `d26a6d1` (plugin unchanged through v6.3.2); branch synced with upstream main 2026-10-25, pending code-owner review | `xai-org/plugin-marketplace#1520`, pinned to a commit SHA; a plugin change needs a new pin. |

## Glama check

Glama builds its own image from a form (base image, build steps, CMD, env schema, placeholders), wraps the CMD with `mcp-proxy`, and checks `initialize` + `tools/list` over stdio.

[`contrib/glama/form.json`](../contrib/glama/form.json) holds the values to paste:

1. Install PostgreSQL 17 and pgvector from Debian trixie main, plus `curl` and `jq`.
2. Pull the `ghcr.io/thek3nsai/ops-brain:latest` linux/amd64 binary: resolve the manifest, verify the manifest and layer digests, and take `app/ops-brain` from the topmost layer that has it. A missing binary fails the build.
3. Install [`ops-brain-glama.sh`](../contrib/glama/ops-brain-glama.sh) from the clone at `/app`. It starts a throwaway trust-auth Postgres on loopback, creates the `vector` extension, unsets every token variable, and execs ops-brain on stdio with embeddings off.

The placeholders are dummies the script overrides. The check runs the latest *released* binary, not the synced commit, so between a merge and the next tag it tests the previous release.

## On each release

After the `v*` tag's release workflow is green (the MCP Registry publishes itself) and prod is deployed, Eduardo clicks **Repository → Sync Server** on Glama's admin page. Auto-release is on, so Glama builds and publishes by itself and emails k3nsai@gmail.com "Build succeeded" and "Release X published". No manual digest update or Make Release step.

Verify with those emails, not the page changelog, which lags and lists schema changes only.
