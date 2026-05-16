# ops-brain Roadmap

What we build, what we don't, why. Philosophy first. Shipped-work history lives in `CHANGELOG.md`.

## Philosophy (read this first, every time)

**ops-brain is the team bus, not a brain.** Local is the source of truth — each
agent's per-machine instructions are its scope, its filesystem is its state, its
git history is its memory. ops-brain exists for the handful of things agents
genuinely cannot do alone: handoffs between machines, shared incidents,
cross-client knowledge with isolation rules, tickets that span systems.
**If a question can be answered without ops-brain, it should be.**

**We kill without mercy.** Any tool, feature, or scaffolding that an agent can
work around by just doing the work locally gets deleted, not deferred. "Wait for
demand" is a lie we tell ourselves before building it anyway on a quiet
afternoon. A feature is either load-bearing for cross-agent coordination today,
or it's dead.

**Killer features only.** The test before building anything new: *does this
solve a problem we've actually hit, or does it let agents coordinate a thing
they genuinely cannot coordinate locally?* If the answer is "it would be nice
for a 4-person team" or "it would be nice to measure X," don't build it.

**Measurement is management ceremony, not a killer feature.** If we're not
bleeding from a metric's absence, we don't build the metric. Management
dashboards, deploy latency histograms, and workflow chains all live on the
dead list.

**Every tool costs tokens across all agent instances on every session.** Tool
bloat is the main cost center. Every added field on `check_in`, every new tool
stub, every extra sentence in `get_info` dilutes the briefing for everyone.
Guard the surface area aggressively.

The product bar is also codified in ops-brain knowledge entry
`019e0d79-3a7f-7902-86cc-db4a573c1071` (cross-client-safe, global) — reference
it when evaluating any new feature.

## Hard stops (don't build these, ever)

Non-negotiable. Any idea that violates one of these is DOA.

- **No scheduling/orchestration features.** cron, systemd timers, Ansible,
  Task Scheduler, and the existing `schedule` skill cover this. ops-brain
  owns memory + coordination, never execution timing.
- **No session management.** `start_session`/`end_session` were removed in
  v1.3. Don't add features that require per-session state.
- **No new fields on `check_in`.** It's the right size now. Every added field
  dilutes the briefing for every call, for every agent, forever.
- **No generic wiki / documentation features.** Local docs + GitHub are truth.
  ops-brain knowledge is strictly for cross-agent gotchas that cross scope
  boundaries.
- **No structured-evidence handoff types.** Handoffs stay freeform markdown
  forever.
- **No workflow chains on handoffs.** No `workflow_id`, no `list_workflows`,
  no deploy-chain measurement.
- **No identity storage server-side.** `cc_identities` was dropped in v1.5 for
  good reason — identity lives in each agent's per-machine instructions, full
  stop. Don't reintroduce it under any other name.
- **No server-side preferences table.** `preferences` was dropped in v1.5 for
  good reason — per-agent config belongs in each agent's local instructions or
  in per-call parameters, not in a shared row that mutates other agents'
  defaults.
- **No inventory, incidents, or monitoring tables.** Removed in v3.0.0.
  Configuration management owns inventory, Zammad owns tickets/incidents,
  Uptime Kuma owns monitoring. ops-brain stays on its lane.

## Dead forever (don't resurrect)

These are not "wait for demand." They are dead. If real friction ever surfaces
that demands them back, re-open the question from first principles — do not
resurrect these designs.

### ❌ Deploy-chain workflow

**What it would have been:** `workflow_id` on handoffs linking them into chains
(HSR → Stealth → Cloud → HSR), plus a `list_workflows` view to measure deploy
latency.

**Why dead:** measurement is management ceremony. The natural workflow is
working. Imposing scaffolding to measure latency we aren't bleeding from is the
exact "structure for structure's sake" the team-bus reframe rejects.

### ❌ Investigation handoffs with structured evidence

**What it would have been:** a structured handoff type carrying log excerpts,
query links, config snippets, hypothesis timelines as first-class fields.

**Why dead:** handoffs already carry freeform markdown bodies. If an agent
wants to attach logs, query links, or a hypothesis timeline, it pastes them
into the body — the way every real handoff in this repo already works. Adding
structured fields competes with markdown and loses.

### ❌ Runbooks

Killed in v1.8.0 (2026-04-26). 27 prod runbooks triaged → zero migrated to
`knowledge`. Net +106/-1707 with no functional regression. Established the
"drift, not feature" precedent: when a surface pulls against
local-as-source-of-truth, **remove it**.

### ❌ Inventory / incidents / monitoring tables

Removed in v3.0.0 (2026-05-09). Tool count 59 → 18, ~ -10k LOC, 13 tables
CASCADE-dropped. See `CHANGELOG.md` for the full record.

## Open-but-dormant items

Not on the roadmap, noted for when they hit.

- **Zammad retirement audit.** Eduardo confirmed Zammad is supposed to go
  away. ops-brain tickets are the survivor. Before shutdown, someone needs to
  audit what's in Zammad and decide what should migrate. Not on the roadmap
  because there's no date; flag it when the shutdown date lands.

## How to apply this file

When sitting down to work on ops-brain **features** (not bug fixes, not security
patches, not cosmetic polish):

1. Read this file first, starting at the philosophy section at the top.
2. If you have a new feature idea, run it through the hard-stops list AND the
   "does this solve a real problem we've hit" test. If it doesn't pass both,
   don't build it.
3. Do NOT resurrect a dead item without re-opening the question from first
   principles. "Wait for demand" is not the same as "dead."
4. As of v3.1.0, ops-brain is in operator mode — new features should be the
   exception, removals the norm. Read `CHANGELOG.md` for shipped history.
