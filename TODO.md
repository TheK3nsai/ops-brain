# TODO

## v3.0.0 De-bloat (done)

- [x] Remove inventory subsystem (servers, services, sites, networks, vendors)
- [x] Remove incidents subsystem (incidents, ticket_links, monitor links)
- [x] Remove monitoring/watchdog subsystem
- [x] Drop `knowledge.source_incident_id` (provenance lives in `author` only)

## Follow-ups

- [ ] Verify prod data has been exported/archived before applying migration `20260509122935_drop_inventory_and_incidents.sql`
- [ ] Watch the first post-deploy briefing to confirm handoffs+tickets sections still render cleanly
