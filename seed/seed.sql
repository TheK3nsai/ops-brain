-- ops-brain seed data — clients only
-- Last updated: 2026-07-02 (v4.0.0 Zammad retirement)
--
-- v3.0.0: inventory tables (sites, networks, servers, services, vendors)
-- and incidents/monitors are gone. v4.0.0: Zammad ticketing retired too.
-- The only foundational data ops-brain needs is the client list — used to
-- scope knowledge entries. Everything else is owned by config
-- management or the client's own systems.
--
-- Knowledge entries and handoffs are populated by live agents at runtime.
-- The two clients below are neutral examples; replace them with your own.
-- Do NOT add other placeholder data here.

INSERT INTO clients (id, name, slug, notes) VALUES
    ('019594a0-0001-7000-8000-000000000003', 'Acme Corp', 'acme', 'Example client'),
    ('019594a0-0001-7000-8000-000000000004', 'Globex', 'globex', 'Example client')
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes;
