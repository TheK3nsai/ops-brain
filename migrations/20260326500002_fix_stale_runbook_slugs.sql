-- Fix stale slugs from seed data where titles were updated but slugs weren't.
-- Slugs are the public lookup key — they should reflect the current title.
UPDATE runbooks SET slug = 'backup-infrastructure' WHERE slug = 'veeam-backup-failure';
UPDATE runbooks SET slug = 'backup-job-conflict' WHERE slug = 'veeam-backup-conflict';
UPDATE runbooks SET slug = 'vm-disaster-recovery' WHERE slug = 'hyperv-vm-failover';
UPDATE runbooks SET slug = 'disk-space-emergency' WHERE slug = 'file-server-disk-space';
