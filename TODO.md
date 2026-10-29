# TODO

Open work only, a few lines each. Shipped history lives in `CHANGELOG.md`; doctrine, hard stops and decided-don't-re-propose items in `ROADMAP.md`; code and deploy footguns in `GOTCHAS.md`. Long working notes for an open item go in `docs/investigations/` and are deleted with the item.

**Release intent:** prioritize the existing fleet. Client compatibility and optional self-hosting guidance are authorized; do not add public-service or multi-tenant infrastructure without a separate need.

## Open

### Bus core

- **`operator-notify.sh` mails an item once and then goes quiet.** The briefing's "waiting on you" section is the standing view, so decide whether the script needs a re-reminder at all once that has been lived with.

### Distribution

- **OpenAI submission validation.** Validate the portable OAuth ZIP in the
  submission portal before public submission. Real Auth0 connections and
  ordinary model-selected handoffs have passed with installed Codex and Grok
  plugins. The existing fleet is the test cohort; outside beta users are not
  a prerequisite. Verified scope and fixture commands: `docs/oauth.md`.
- **Phase 2 listings: follow up on review.** Grok Build catalog PR `xai-org/plugin-marketplace#1520` is synced with upstream main and awaits code-owner review; Docker MCP Catalog `docker/mcp-registry#5871` awaits a maintainer's CI approval. The GitHub MCP Registry has accepted the entry. State lives in `docs/listings.md`; drop this item once both pending listings are live or declined.

### Public-repo hygiene

- **String guard residuals.** Messages are now guarded before commit (`.githooks/commit-msg`, once `core.hooksPath` is set per clone) and CI scans PR text and message ranges. PR bodies are prevented locally only where an agent hook covers `gh`/MCP writes; that is Claude Code on the operator workstation, so Codex and humans still rely on CI detection, and an edit after the fact is cosmetic (the host keeps prior revisions). CI scans PR text only: issues, PR comments, reviews and release notes get no CI scan at all, just that Claude hook. Also: an unmergeable PR runs no `pull_request` workflow, so it is unscanned until rebased. And the guarded classes are host and client identifiers only — a person's name in a fixture or example passes; add a class if that matters.
- **Pre-scrub residue is reachable, and a GC request is the wrong lever** — the same class of string is permanently in merged history, which GC never touches. Operator's call between accepting the exposure fleet-wide or planning a history rewrite; don't file the GC request as a standalone. (The related migration waiver is decided: `ROADMAP.md`.)
