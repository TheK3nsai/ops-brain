# Gotchas

## Database Migrations

- **Inventory and incident tables were dropped in v3.0.0.** Do not add them back. Configuration management (Terraform/Ansible/local config files) is the source of truth for inventory; Zammad is the source of truth for tickets/incidents; Uptime Kuma is the source of truth for monitoring. ops-brain stays on its lane: handoffs, knowledge, briefings, Zammad orchestration.
- **`knowledge.source_incident_id` was dropped** in the same migration. Provenance now lives entirely in the `author` column.
