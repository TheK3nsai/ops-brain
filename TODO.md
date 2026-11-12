# TODO

Open work only, a few lines each. Shipped history lives in `CHANGELOG.md`; doctrine, hard stops and decided-don't-re-propose items in `ROADMAP.md`; code and deploy footguns in `GOTCHAS.md`. Long working notes for an open item go in `docs/investigations/` and are deleted with the item.

**Release intent:** the existing fleet. Distribution is frozen at what updates itself (`ROADMAP.md` → *Decided*); do not add public-service or multi-tenant infrastructure.

## Open

### Bus core

- **`operator-notify.sh` mails an item once and then goes quiet.** The briefing's "waiting on you" section is the standing view, so decide whether the script needs a re-reminder at all once that has been lived with.

### Tests

- **`scripts/test-oauth-flow.py` times out at fixture startup on a local checkout** ("fixture ops-brain readiness timed out"), with and without `--start-postgres`, and on `main` before v7.0.0 as well. CI passes it. Cause not diagnosed: the script withholds the child's output by design, so start by keeping the fixture's `server.log`.
