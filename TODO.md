# TODO

Open work only, a few lines each. Shipped history lives in `CHANGELOG.md`; doctrine, hard stops and decided-don't-re-propose items in `ROADMAP.md`; code and deploy footguns in `GOTCHAS.md`. Long working notes for an open item go in `docs/investigations/` and are deleted with the item.

**Release intent:** prioritize the existing fleet. Client compatibility and optional self-hosting guidance are authorized; do not add public-service or multi-tenant infrastructure without a separate need.

## Open

### Bus core

- **`operator-notify.sh` mails an item once and then goes quiet.** The briefing's "waiting on you" section is the standing view, so decide whether the script needs a re-reminder at all once that has been lived with.

### Distribution

- **OpenAI submission validation.** Validate the portable OAuth ZIP in the
  submission portal before public submission; upload is currently blocked on
  developer identity review. Final submission also needs listing URLs, a demo
  recording, five positive and three negative review cases, and reviewer access.
  The existing fleet is the test
  cohort; outside beta users are not a prerequisite. Verified scope: `docs/oauth.md`.
- **Phase 2 listings: follow up on review.** Keep the Grok Build catalog branch current and follow up on the Grok and Docker reviews. Current PRs, pins and blockers live in `docs/listings.md`; drop this item once both listings are live or declined.

### Public-repo hygiene

- **Pre-scrub residue is reachable, and a GC request is the wrong lever** — the same class of string is permanently in merged history, which GC never touches. Operator's call between accepting the exposure fleet-wide or planning a history rewrite; don't file the GC request as a standalone. (The related migration waiver is decided: `ROADMAP.md`.)
