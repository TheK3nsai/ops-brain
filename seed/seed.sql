-- ops-brain seed data — foundational structure only
-- Last updated: 2026-03-25
--
-- WARNING: This file only seeds clients, sites, and networks.
-- All servers, services, knowledge, runbooks, vendors, and incidents
-- are populated via MCP tools from live CC sessions against real infrastructure.
-- Do NOT add fictional/placeholder data here.
--
-- Production database is on kensai.cloud (shared-postgres).
-- All CC instances connect via HTTP — this is the single source of truth.

-- Clients
INSERT INTO clients (id, name, slug, notes, zammad_org_id, zammad_group_id, zammad_customer_id) VALUES
    ('019594a0-0001-7000-8000-000000000001', 'HSR-PR (Hospice)', 'hsr', 'Hospice del Sureste / Renacer, ~300 employees, ~400 patients', 2, 2, 5),
    ('019594a0-0001-7000-8000-000000000002', 'CPA Firm', 'cpa', 'Small CPA firm, 4 employees, hundreds of tax season customers', 3, 4, 6)
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes,
    zammad_org_id = EXCLUDED.zammad_org_id, zammad_group_id = EXCLUDED.zammad_group_id, zammad_customer_id = EXCLUDED.zammad_customer_id;

-- Sites
INSERT INTO sites (id, client_id, name, slug, address, wan_provider, notes) VALUES
    ('019594a0-0002-7000-8000-000000000001', '019594a0-0001-7000-8000-000000000001', 'HSR Main Office', 'hsr-main', NULL, NULL, 'Primary hospice location (Cabo Rojo)'),
    ('019594a0-0002-7000-8000-000000000002', '019594a0-0001-7000-8000-000000000001', 'HSR Renacer', 'hsr-renacer', NULL, NULL, 'Second hospice location — no dedicated servers, connects via site-to-site VPN'),
    ('019594a0-0002-7000-8000-000000000003', '019594a0-0001-7000-8000-000000000001', 'HSR Cloud', 'hsr-cloud', NULL, NULL, 'Cloud/remote infrastructure'),
    ('019594a0-0002-7000-8000-000000000004', '019594a0-0001-7000-8000-000000000002', 'CPA Office', 'cpa-office', NULL, NULL, 'CPA firm office'),
    ('019594a0-0002-7000-8000-000000000005', '019594a0-0001-7000-8000-000000000002', 'Eduardo Home/Remote', 'eduardo-remote', NULL, NULL, 'Eduardo personal/work machines')
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes;

-- Networks
INSERT INTO networks (id, site_id, name, cidr, vlan_id, gateway, dns_servers, purpose) VALUES
    ('019594a0-0005-7000-8000-000000000001', '019594a0-0002-7000-8000-000000000001', 'HSR Main LAN', '10.10.1.0/24', NULL, '10.10.1.1', '{10.10.1.206}', 'Main office network — SR-SERVER is sole DNS'),
    ('019594a0-0005-7000-8000-000000000002', '019594a0-0002-7000-8000-000000000002', 'HSR Renacer LAN', '10.10.2.0/24', NULL, '10.10.2.1', '{10.10.1.206}', 'Renacer office — depends on main site DC via VPN'),
    ('019594a0-0005-7000-8000-000000000003', '019594a0-0002-7000-8000-000000000005', 'Eduardo Home LAN', '10.88.223.0/24', NULL, '10.88.223.1', '{1.1.1.1,9.9.9.9}', 'Home network')
ON CONFLICT DO NOTHING;
