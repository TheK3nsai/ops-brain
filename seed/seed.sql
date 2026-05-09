-- ops-brain seed data — clients only
-- Last updated: 2026-05-09 (v3.0.0 de-bloat)
--
-- v3.0.0: inventory tables (sites, networks, servers, services, vendors)
-- and incidents/monitors are gone. The only foundational data ops-brain needs
-- is the client list — used to scope knowledge entries, briefings, and
-- Zammad ticket creation. Everything else is owned by config management,
-- Zammad, or Uptime Kuma.
--
-- Knowledge entries and handoffs are populated by live agents at runtime.
-- Do NOT add fictional/placeholder data here.

INSERT INTO clients (id, name, slug, notes, zammad_org_id, zammad_group_id, zammad_customer_id) VALUES
    ('019594a0-0001-7000-8000-000000000001', 'Acme Healthcare', 'acme-health', 'Regional healthcare provider, ~300 employees, ~400 patients', 2, 2, 5),
    ('019594a0-0001-7000-8000-000000000002', 'Summit CPA', 'summit-cpa', 'Small accounting firm, 4 employees, seasonal tax customers', 3, 4, 6)
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes,
    zammad_org_id = EXCLUDED.zammad_org_id, zammad_group_id = EXCLUDED.zammad_group_id, zammad_customer_id = EXCLUDED.zammad_customer_id;
