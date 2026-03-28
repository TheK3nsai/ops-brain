-- ops-brain seed data — foundational structure only
-- Last updated: 2026-03-28
--
-- WARNING: This file only seeds clients, sites, and networks.
-- All servers, services, knowledge, runbooks, vendors, and incidents
-- are populated via MCP tools from live CC sessions against real infrastructure.
-- Do NOT add fictional/placeholder data here.

-- Clients
INSERT INTO clients (id, name, slug, notes, zammad_org_id, zammad_group_id, zammad_customer_id) VALUES
    ('019594a0-0001-7000-8000-000000000001', 'Acme Healthcare', 'acme-health', 'Regional healthcare provider, ~300 employees, ~400 patients', 2, 2, 5),
    ('019594a0-0001-7000-8000-000000000002', 'Summit CPA', 'summit-cpa', 'Small accounting firm, 4 employees, seasonal tax customers', 3, 4, 6)
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes,
    zammad_org_id = EXCLUDED.zammad_org_id, zammad_group_id = EXCLUDED.zammad_group_id, zammad_customer_id = EXCLUDED.zammad_customer_id;

-- Sites
INSERT INTO sites (id, client_id, name, slug, address, wan_provider, notes) VALUES
    ('019594a0-0002-7000-8000-000000000001', '019594a0-0001-7000-8000-000000000001', 'Acme Main Office', 'acme-main', NULL, NULL, 'Primary healthcare location'),
    ('019594a0-0002-7000-8000-000000000002', '019594a0-0001-7000-8000-000000000001', 'Acme Satellite', 'acme-satellite', NULL, NULL, 'Second location — no dedicated servers, connects via site-to-site VPN'),
    ('019594a0-0002-7000-8000-000000000003', '019594a0-0001-7000-8000-000000000001', 'Acme Cloud', 'acme-cloud', NULL, NULL, 'Cloud/remote infrastructure'),
    ('019594a0-0002-7000-8000-000000000004', '019594a0-0001-7000-8000-000000000002', 'Summit Office', 'summit-office', NULL, NULL, 'CPA firm office'),
    ('019594a0-0002-7000-8000-000000000005', '019594a0-0001-7000-8000-000000000002', 'Admin Remote', 'admin-remote', NULL, NULL, 'IT admin personal/work machines')
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes;

-- Networks
INSERT INTO networks (id, site_id, name, cidr, vlan_id, gateway, dns_servers, purpose) VALUES
    ('019594a0-0005-7000-8000-000000000001', '019594a0-0002-7000-8000-000000000001', 'Acme Main LAN', '10.10.1.0/24', NULL, '10.10.1.1', '{10.10.1.10}', 'Main office network — primary DC is sole DNS'),
    ('019594a0-0005-7000-8000-000000000002', '019594a0-0002-7000-8000-000000000002', 'Acme Satellite LAN', '10.10.2.0/24', NULL, '10.10.2.1', '{10.10.1.10}', 'Satellite office — depends on main site DC via VPN'),
    ('019594a0-0005-7000-8000-000000000003', '019594a0-0002-7000-8000-000000000005', 'Admin Home LAN', '10.20.0.0/24', NULL, '10.20.0.1', '{1.1.1.1,9.9.9.9}', 'Home network')
ON CONFLICT DO NOTHING;
