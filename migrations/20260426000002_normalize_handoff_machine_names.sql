-- v1.9 — Normalize handoff machine names to canonical CC form.
--
-- Background: handoffs.from_machine and handoffs.to_machine were free-text
-- and accumulated a mix of hostnames (`stealth`, `kensai-cloud`, `HV-FS0`,
-- `SMYT-SERVER`, occasionally `CPA-SRV`) and CC names (`CC-Stealth`,
-- `CC-CPA`, …). The mismatch silently broke `check_in` routing — a CC
-- writing `to_machine = "CC-Stealth"` was invisible to a recipient
-- querying by hostname `stealth`.
--
-- After this migration all rows store the canonical CC name. Tool handlers
-- (handle_create_handoff, handle_list_handoffs, handle_check_in,
-- handle_get_situational_awareness) normalize at the boundary so callers
-- can still pass either form and converge on the CC name.
--
-- Idempotent: rerunning is a no-op because canonical CC values
-- (`CC-Cloud`, `CC-Stealth`, `CC-HSR`, `CC-CPA`) are not in the WHEN list.
-- LOWER() on the input handles mixed-case writes (`hv-fs0`, `Stealth`).
-- Unknown values pass through unchanged via ELSE.

UPDATE handoffs
SET
    from_machine = CASE LOWER(from_machine)
        WHEN 'stealth' THEN 'CC-Stealth'
        WHEN 'kensai-cloud' THEN 'CC-Cloud'
        WHEN 'hv-fs0' THEN 'CC-HSR'
        WHEN 'smyt-server' THEN 'CC-CPA'
        WHEN 'cpa-srv' THEN 'CC-CPA'
        ELSE from_machine
    END,
    to_machine = CASE LOWER(to_machine)
        WHEN 'stealth' THEN 'CC-Stealth'
        WHEN 'kensai-cloud' THEN 'CC-Cloud'
        WHEN 'hv-fs0' THEN 'CC-HSR'
        WHEN 'smyt-server' THEN 'CC-CPA'
        WHEN 'cpa-srv' THEN 'CC-CPA'
        ELSE to_machine
    END
WHERE
    LOWER(from_machine) IN ('stealth', 'kensai-cloud', 'hv-fs0', 'smyt-server', 'cpa-srv')
    OR LOWER(to_machine) IN ('stealth', 'kensai-cloud', 'hv-fs0', 'smyt-server', 'cpa-srv');
