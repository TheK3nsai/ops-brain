-- ops-brain seed data — clients only
-- Last updated: 2026-07-02 (v4.0.0 Zammad retirement)
--
-- v3.0.0: inventory tables (sites, networks, servers, services, vendors)
-- and incidents/monitors are gone. v4.0.0: Zammad ticketing retired too.
-- The only foundational data ops-brain needs is the client list — used to
-- scope knowledge entries and briefings. Everything else is owned by config
-- management or the client's own systems.
--
-- Knowledge entries and handoffs are populated by live agents at runtime.
-- Do NOT add fictional/placeholder data here.

INSERT INTO clients (id, name, slug, notes) VALUES
    ('019594a0-0001-7000-8000-000000000001', 'Acme Healthcare', 'acme-health', 'Regional healthcare provider, ~300 employees, ~400 patients'),
    ('019594a0-0001-7000-8000-000000000002', 'Summit CPA', 'summit-cpa', 'Small accounting firm, 4 employees, seasonal tax customers')
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes;
