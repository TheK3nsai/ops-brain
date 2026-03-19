-- ops-brain seed data
-- Eduardo's actual infrastructure

-- Clients
INSERT INTO clients (id, name, slug, notes) VALUES
    ('019594a0-0001-7000-8000-000000000001', 'HSR-PR (Hospice)', 'hsr', 'Hospice del Sureste / Renacer, ~300 employees, ~400 patients'),
    ('019594a0-0001-7000-8000-000000000002', 'CPA Firm', 'cpa', 'Small CPA firm, 4 employees, hundreds of tax season customers')
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes;

-- Sites
INSERT INTO sites (id, client_id, name, slug, address, wan_provider, notes) VALUES
    ('019594a0-0002-7000-8000-000000000001', '019594a0-0001-7000-8000-000000000001', 'HSR Main Office', 'hsr-main', NULL, NULL, 'Primary hospice location'),
    ('019594a0-0002-7000-8000-000000000002', '019594a0-0001-7000-8000-000000000001', 'HSR Renacer', 'hsr-renacer', NULL, NULL, 'Second hospice location'),
    ('019594a0-0002-7000-8000-000000000003', '019594a0-0001-7000-8000-000000000001', 'HSR Cloud', 'hsr-cloud', NULL, NULL, 'Cloud/remote infrastructure for HSR'),
    ('019594a0-0002-7000-8000-000000000004', '019594a0-0001-7000-8000-000000000002', 'CPA Office', 'cpa-office', NULL, NULL, 'CPA firm office'),
    ('019594a0-0002-7000-8000-000000000005', '019594a0-0001-7000-8000-000000000002', 'Eduardo Home/Remote', 'eduardo-remote', NULL, NULL, 'Eduardo personal/work machines')
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, notes = EXCLUDED.notes;

-- Servers (HSR)
INSERT INTO servers (id, site_id, hostname, slug, os, ip_addresses, roles, status, notes, is_virtual) VALUES
    ('019594a0-0003-7000-8000-000000000001', '019594a0-0002-7000-8000-000000000001', 'HV-DC0', 'hvdc0', 'Windows Server 2022', '{10.10.1.50}', '{hyper-v-host,domain-controller}', 'active', 'Primary DC and Hyper-V host', false),
    ('019594a0-0003-7000-8000-000000000002', '019594a0-0002-7000-8000-000000000001', 'HV-FS0', 'hvfs0', 'Windows Server 2022', '{10.10.1.54}', '{hyper-v-host,file-server}', 'active', 'File server and Hyper-V host', false),
    ('019594a0-0003-7000-8000-000000000003', '019594a0-0002-7000-8000-000000000001', 'HV-APP0', 'hvapp0', 'Windows Server 2022', '{10.10.1.55}', '{hyper-v-host,app-server}', 'active', 'Application server and Hyper-V host', false),
    ('019594a0-0003-7000-8000-000000000004', '019594a0-0002-7000-8000-000000000002', 'HV-DC1', 'hvdc1', 'Windows Server 2022', '{10.10.2.50}', '{hyper-v-host,domain-controller}', 'active', 'Renacer DC and Hyper-V host', false),
    ('019594a0-0003-7000-8000-000000000005', '019594a0-0002-7000-8000-000000000002', 'HV-RDP1', 'hvrdp1', 'Windows Server 2022', '{10.10.2.51}', '{hyper-v-host,rdp-server}', 'active', 'Renacer RDP server', false),
    ('019594a0-0003-7000-8000-000000000006', '019594a0-0002-7000-8000-000000000003', 'HSR-WEB', 'hsr-web', 'Ubuntu 22.04', '{}', '{web-server}', 'active', 'HSR cloud web server', true)
ON CONFLICT (slug) DO UPDATE SET hostname = EXCLUDED.hostname, os = EXCLUDED.os, ip_addresses = EXCLUDED.ip_addresses, roles = EXCLUDED.roles, status = EXCLUDED.status, notes = EXCLUDED.notes;

-- Servers (CPA)
INSERT INTO servers (id, site_id, hostname, slug, os, ip_addresses, roles, status, notes, is_virtual) VALUES
    ('019594a0-0003-7000-8000-000000000010', '019594a0-0002-7000-8000-000000000004', 'CPA-SRV', 'cpa-srv', 'Windows Server 2022', '{192.168.1.10}', '{file-server,app-server}', 'active', 'CPA main server', false)
ON CONFLICT (slug) DO UPDATE SET hostname = EXCLUDED.hostname, os = EXCLUDED.os, ip_addresses = EXCLUDED.ip_addresses, roles = EXCLUDED.roles, status = EXCLUDED.status, notes = EXCLUDED.notes;

-- Servers (Eduardo)
INSERT INTO servers (id, site_id, hostname, slug, os, ip_addresses, roles, status, notes, is_virtual, cpu, ram_gb, storage_summary, hardware) VALUES
    ('019594a0-0003-7000-8000-000000000020', '019594a0-0002-7000-8000-000000000005', 'stealth', 'stealth', 'Gentoo Linux', '{10.88.223.0/24}', '{workstation,management}', 'active', 'Eduardo primary laptop — MSI GS76 Stealth', false, 'Intel i9-11900H 8C/16T', 32, '954GB NVMe XFS', 'MSI GS76 Stealth 11UG'),
    ('019594a0-0003-7000-8000-000000000021', '019594a0-0002-7000-8000-000000000003', 'kensai-cloud', 'kensai-cloud', 'Rocky Linux 10.1', '{}', '{cloud-server,docker-host}', 'active', 'VPS — Nextcloud, Zammad, ops-brain, etc.', true, '4 vCPUs', 7, '151GB', 'VPS')
ON CONFLICT (slug) DO UPDATE SET hostname = EXCLUDED.hostname, os = EXCLUDED.os, ip_addresses = EXCLUDED.ip_addresses, roles = EXCLUDED.roles, status = EXCLUDED.status, notes = EXCLUDED.notes;

-- Services
INSERT INTO services (id, name, slug, category, description, criticality) VALUES
    ('019594a0-0004-7000-8000-000000000001', 'Active Directory', 'active-directory', 'identity', 'Domain controller services', 'critical'),
    ('019594a0-0004-7000-8000-000000000002', 'DNS', 'dns', 'network', 'Internal DNS resolution', 'critical'),
    ('019594a0-0004-7000-8000-000000000003', 'DHCP', 'dhcp', 'network', 'Dynamic IP assignment', 'high'),
    ('019594a0-0004-7000-8000-000000000004', 'Hyper-V', 'hyper-v', 'virtualization', 'Hypervisor for VMs', 'critical'),
    ('019594a0-0004-7000-8000-000000000005', 'File Shares (SMB)', 'file-shares', 'storage', 'Windows file sharing', 'high'),
    ('019594a0-0004-7000-8000-000000000006', 'RDP Services', 'rdp', 'remote-access', 'Remote Desktop Protocol', 'high'),
    ('019594a0-0004-7000-8000-000000000007', 'Veeam Backup', 'veeam', 'backup', 'Backup and disaster recovery', 'critical'),
    ('019594a0-0004-7000-8000-000000000008', 'Splashtop', 'splashtop', 'remote-access', 'Remote support tool', 'medium'),
    ('019594a0-0004-7000-8000-000000000009', 'Nextcloud', 'nextcloud', 'collaboration', 'File sync and collaboration', 'medium'),
    ('019594a0-0004-7000-8000-000000000010', 'Zammad', 'zammad', 'ticketing', 'Help desk / ticketing system', 'medium'),
    ('019594a0-0004-7000-8000-000000000011', 'Docker', 'docker', 'containers', 'Container runtime', 'high'),
    ('019594a0-0004-7000-8000-000000000012', 'Caddy', 'caddy', 'web', 'Reverse proxy', 'high'),
    ('019594a0-0004-7000-8000-000000000013', 'Print Services', 'print', 'printing', 'Network printing', 'low'),
    ('019594a0-0004-7000-8000-000000000014', 'Group Policy', 'gpo', 'management', 'Windows Group Policy management', 'high'),
    ('019594a0-0004-7000-8000-000000000015', 'Cloudflare WARP', 'warp', 'vpn', 'Zero Trust VPN tunnel', 'high')
ON CONFLICT (slug) DO UPDATE SET name = EXCLUDED.name, category = EXCLUDED.category, description = EXCLUDED.description, criticality = EXCLUDED.criticality;

-- Server-Service links
INSERT INTO server_services (server_id, service_id, port, config_notes) VALUES
    -- HV-DC0: AD, DNS, DHCP, Hyper-V, GPO
    ('019594a0-0003-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000001', NULL, 'Primary domain controller'),
    ('019594a0-0003-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000002', 53, 'AD-integrated DNS'),
    ('019594a0-0003-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000003', NULL, 'Main office DHCP'),
    ('019594a0-0003-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000004', NULL, NULL),
    ('019594a0-0003-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000014', NULL, NULL),
    -- HV-FS0: File Shares, Hyper-V, Veeam
    ('019594a0-0003-7000-8000-000000000002', '019594a0-0004-7000-8000-000000000005', 445, 'Primary file server'),
    ('019594a0-0003-7000-8000-000000000002', '019594a0-0004-7000-8000-000000000004', NULL, NULL),
    ('019594a0-0003-7000-8000-000000000002', '019594a0-0004-7000-8000-000000000007', NULL, 'Veeam backup server'),
    -- HV-APP0: Hyper-V, app hosting
    ('019594a0-0003-7000-8000-000000000003', '019594a0-0004-7000-8000-000000000004', NULL, NULL),
    -- HV-DC1: AD, DNS, DHCP, Hyper-V (Renacer)
    ('019594a0-0003-7000-8000-000000000004', '019594a0-0004-7000-8000-000000000001', NULL, 'Renacer domain controller'),
    ('019594a0-0003-7000-8000-000000000004', '019594a0-0004-7000-8000-000000000002', 53, 'Renacer DNS'),
    ('019594a0-0003-7000-8000-000000000004', '019594a0-0004-7000-8000-000000000003', NULL, 'Renacer DHCP'),
    ('019594a0-0003-7000-8000-000000000004', '019594a0-0004-7000-8000-000000000004', NULL, NULL),
    -- HV-RDP1: RDP, Hyper-V
    ('019594a0-0003-7000-8000-000000000005', '019594a0-0004-7000-8000-000000000006', 3389, 'Renacer RDP host'),
    ('019594a0-0003-7000-8000-000000000005', '019594a0-0004-7000-8000-000000000004', NULL, NULL),
    -- kensai-cloud: Docker, Caddy, Nextcloud, Zammad
    ('019594a0-0003-7000-8000-000000000021', '019594a0-0004-7000-8000-000000000011', NULL, 'Docker host'),
    ('019594a0-0003-7000-8000-000000000021', '019594a0-0004-7000-8000-000000000012', 8080, 'Caddy reverse proxy'),
    ('019594a0-0003-7000-8000-000000000021', '019594a0-0004-7000-8000-000000000009', NULL, 'cloud.kensai.cloud'),
    ('019594a0-0003-7000-8000-000000000021', '019594a0-0004-7000-8000-000000000010', NULL, 'support.kensai.cloud')
ON CONFLICT (server_id, service_id) DO UPDATE SET port = EXCLUDED.port, config_notes = EXCLUDED.config_notes;

-- Networks
INSERT INTO networks (id, site_id, name, cidr, vlan_id, gateway, dns_servers, purpose) VALUES
    ('019594a0-0005-7000-8000-000000000001', '019594a0-0002-7000-8000-000000000001', 'HSR Main LAN', '10.10.1.0/24', NULL, '10.10.1.1', '{10.10.1.50}', 'Main office network'),
    ('019594a0-0005-7000-8000-000000000002', '019594a0-0002-7000-8000-000000000002', 'HSR Renacer LAN', '10.10.2.0/24', NULL, '10.10.2.1', '{10.10.2.50}', 'Renacer office network'),
    ('019594a0-0005-7000-8000-000000000003', '019594a0-0002-7000-8000-000000000005', 'Eduardo Home LAN', '10.88.223.0/24', NULL, '10.88.223.1', '{1.1.1.1,9.9.9.9}', 'Home network')
ON CONFLICT DO NOTHING;

-- Knowledge entries
INSERT INTO knowledge (id, title, content, category, tags, client_id) VALUES
    ('019594a0-0006-7000-8000-000000000001', 'Splashtop Remote Access', 'Eduardo uses Splashtop Business for remote access to all client machines. Runs via Wine on Gentoo Linux. Desktop entry at ~/.local/share/applications/splashtop-business.desktop.', 'remote-access', '{splashtop,wine,remote}', NULL),
    ('019594a0-0006-7000-8000-000000000002', 'HSR Network Architecture', 'HSR has two sites connected via site-to-site VPN. Main office is 10.10.1.0/24, Renacer is 10.10.2.0/24. Each site has its own domain controller. All servers are physical with Hyper-V role.', 'networking', '{hsr,network,vpn}', '019594a0-0001-7000-8000-000000000001'),
    ('019594a0-0006-7000-8000-000000000003', 'kensai.cloud Stack', 'Rocky Linux 10.1 VPS running Docker. Caddy reverse proxy (replaced Traefik). cloudflared tunnel for Cloudflare connectivity. Services: Nextcloud, Zammad, InvoicePlane, Collabora, Homer. PostgreSQL 18 and MariaDB 11.8 for databases. SSH via ssh.kensai.cloud:22022.', 'infrastructure', '{cloud,docker,caddy}', NULL),
    ('019594a0-0006-7000-8000-000000000004', 'WARP VPN for HSR Access', 'Cloudflare WARP / Zero Trust provides VPN tunnel to HSR servers. All 6 hospice servers accessible via direct SSH through WARP tunnel. Configured on stealth (Gentoo laptop).', 'vpn', '{warp,cloudflare,hsr,vpn}', '019594a0-0001-7000-8000-000000000001'),
    ('019594a0-0006-7000-8000-000000000005', 'Eduardo Role and Responsibilities', 'Solo IT Director (MSP/consultant). Manages two clients: HSR hospice (~300 employees, ~400 patients, 2 locations) and a CPA firm (4 employees). Uses CC across 3 machines: stealth (Gentoo laptop), HSR servers, kensai.cloud VPS.', 'operations', '{role,msp,management}', NULL)
ON CONFLICT DO NOTHING;

-- Sample runbook
INSERT INTO runbooks (id, title, slug, category, content, version, tags, estimated_minutes, requires_reboot) VALUES
    ('019594a0-0007-7000-8000-000000000001', 'Veeam Backup Conflict Resolution', 'veeam-backup-conflict', 'backup', E'# Veeam Backup Conflict Resolution\n\n## Symptoms\n- SMB timeouts during backup window\n- File share access slow or unresponsive\n- Veeam job overlapping with other operations\n\n## Steps\n1. Check Veeam job schedule in Veeam console\n2. Verify no overlapping backup jobs\n3. Check HV-FS0 resource usage during backup window\n4. Adjust backup schedule if conflicts found\n5. Consider staggering backup jobs across servers\n\n## Prevention\n- Keep backup jobs at least 2 hours apart\n- Monitor HV-FS0 disk I/O during backups\n- Set up alerts for backup job failures', 1, '{veeam,backup,smb,troubleshooting}', 30, false)
ON CONFLICT (slug) DO UPDATE SET content = EXCLUDED.content, version = runbooks.version + 1;

-- Link runbook to server and service
INSERT INTO runbook_servers (runbook_id, server_id) VALUES
    ('019594a0-0007-7000-8000-000000000001', '019594a0-0003-7000-8000-000000000002')
ON CONFLICT DO NOTHING;

INSERT INTO runbook_services (runbook_id, service_id) VALUES
    ('019594a0-0007-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000007'),
    ('019594a0-0007-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000005')
ON CONFLICT DO NOTHING;

-- Sample incident
INSERT INTO incidents (id, title, status, severity, client_id, reported_at, resolved_at, symptoms, root_cause, resolution, prevention, time_to_resolve_minutes) VALUES
    ('019594a0-0008-7000-8000-000000000001', 'SMB timeout during backup', 'resolved', 'medium', '019594a0-0001-7000-8000-000000000001', '2026-02-15 09:00:00+00', '2026-02-15 10:30:00+00', 'File shares on HV-FS0 became unresponsive during morning backup window. Users reported slow access to shared drives.', 'Veeam backup job overlapping with another scheduled task caused disk I/O saturation on HV-FS0.', 'Staggered backup jobs by 2 hours. Moved secondary backup to 2 AM.', 'Monitor disk I/O during backup windows. Set up alerts for backup overlap.', 90)
ON CONFLICT DO NOTHING;

INSERT INTO incident_servers (incident_id, server_id) VALUES
    ('019594a0-0008-7000-8000-000000000001', '019594a0-0003-7000-8000-000000000002')
ON CONFLICT DO NOTHING;

INSERT INTO incident_services (incident_id, service_id) VALUES
    ('019594a0-0008-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000005'),
    ('019594a0-0008-7000-8000-000000000001', '019594a0-0004-7000-8000-000000000007')
ON CONFLICT DO NOTHING;

INSERT INTO incident_runbooks (incident_id, runbook_id, usage) VALUES
    ('019594a0-0008-7000-8000-000000000001', '019594a0-0007-7000-8000-000000000001', 'followed')
ON CONFLICT DO NOTHING;
