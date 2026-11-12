# TODO

Open work only, a few lines each. Shipped history lives in `CHANGELOG.md`; doctrine, hard stops and decided-don't-re-propose items in `ROADMAP.md`; code and deploy footguns in `GOTCHAS.md`. Long working notes for an open item go in `docs/investigations/` and are deleted with the item.

**Release intent:** the existing fleet. Distribution is frozen at what updates itself (`ROADMAP.md` → *Decided*); do not add public-service or multi-tenant infrastructure.

## Open

### Bus core

- **`operator-notify.sh` mails an item once and then goes quiet.** The briefing's "waiting on you" section is the standing view, so decide whether the script needs a re-reminder at all once that has been lived with.
- **Retire the knowledge store.** Every entry was triaged on 2026-11-12: the few that are cross-agent move to the operator's reference repo, host-local ones go to their owner's local docs, and the rest are dropped. Once the owners confirm their entries are moved, remove the four knowledge tools, the client-scope guard that exists only for that store, and embeddings (full-text search over handoffs stays). `search_bus` then searches handoffs only. Do not add entries meanwhile.
