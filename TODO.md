# TODO

Open work only, a few lines each. Shipped history lives in `CHANGELOG.md`; doctrine, hard stops and decided-don't-re-propose items in `ROADMAP.md`; code and deploy footguns in `GOTCHAS.md`. Long working notes for an open item go in `docs/investigations/` and are deleted with the item.

**Release intent:** prioritize the existing fleet. Client compatibility and optional self-hosting guidance are authorized; do not add public-service or multi-tenant infrastructure without a separate need.

## Open

### Bus core

- **`operator-notify.sh` mails an item once and then goes quiet.** The briefing's "waiting on you" section is the standing view, so decide whether the script needs a re-reminder at all once that has been lived with.

### Distribution

- **OpenAI submission validation.** Validate the portable OAuth ZIP in the
  submission portal before public submission. The existing fleet is the test
  cohort; outside beta users are not a prerequisite. Verified scope: `docs/oauth.md`.
- **Phase 2 listings: follow up on review.** Grok Build catalog PR `xai-org/plugin-marketplace#1520` is synced with upstream main and awaits code-owner review; Docker MCP Catalog `docker/mcp-registry#5871` awaits a maintainer's CI approval. The GitHub MCP Registry has accepted the entry. State lives in `docs/listings.md`; drop this item once both pending listings are live or declined.

### Public-repo hygiene

- **String guard residuals.** The guarded GitHub publisher scans explicit PR/issue/comment/review/release payloads before sending them; repo instructions require the same preflight for direct MCP/browser writes, which it cannot intercept. Public-text CI also detects new/edited metadata, including conflicting PRs, once its workflow is on the default branch. Direct writes can still bypass preflight, GitHub retains old revisions, and `GITHUB_TOKEN`-created events normally do not trigger scans. Conflicting/stacked PRs still need rebase/retarget for build, tree and commit-range checks. Guarded classes remain host and client identifiers only; a person's name in a fixture or example passes. Details: `GOTCHAS.md` → *Commit workflow*.
- **Pre-scrub residue is reachable, and a GC request is the wrong lever** — the same class of string is permanently in merged history, which GC never touches. Operator's call between accepting the exposure fleet-wide or planning a history rewrite; don't file the GC request as a standalone. (The related migration waiver is decided: `ROADMAP.md`.)
