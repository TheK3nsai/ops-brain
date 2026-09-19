use chrono::{DateTime, Utc};
use serde::Serialize;
use sqlx::PgPool;

use crate::repo::handoff_repo::OpenActionBrief;

/// Default recipient slug for the "waiting on you" section. Callers override it
/// with the `operator` field on `POST /api/briefing`.
pub const DEFAULT_OPERATOR: &str = "Operator";

/// A pending handoff nobody has picked up after this long is stuck, not fresh.
pub const STUCK_PENDING_DAYS: i64 = 3;

/// An accepted handoff still open after this long has stalled mid-flight.
/// There is no `accepted_at` column, so this is measured from `created_at`
/// like everything else — which is why the rendered line says "open Nd" rather
/// than claiming anything about when it was accepted.
pub const STUCK_ACCEPTED_DAYS: i64 = 7;

/// Most items listed under "waiting on you". Past this the section says it is
/// truncated and reports the true total — a page length must never be readable
/// as a count.
const OPERATOR_SECTION_CAP: usize = 50;

/// Most items listed under "stuck", across all groups. The premise of this
/// section is that work piles up, so without a cap a mailed briefing grows
/// without bound. `StuckSection::total` keeps the true count.
const STUCK_SECTION_CAP: usize = 50;

/// Size of the legacy `pending_titles` / `accepted_titles` window.
const LEGACY_TITLE_LIMIT: usize = 20;

/// Label for handoffs filed with no recipient.
const UNADDRESSED: &str = "(unaddressed)";

/// Structured briefing data returned alongside the markdown content.
#[derive(Debug, Serialize)]
pub struct BriefingData {
    pub briefing_type: String,
    pub generated_at: String,
    /// The slug whose queue filled the "waiting on you" section.
    pub operator: String,
    pub handoffs: HandoffSummaryData,
    pub waiting_on_you: OperatorQueue,
    pub stuck: StuckSection,
    pub counts: CountsSection,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct HandoffSummaryData {
    pub open_count: usize,
    pub pending_count: usize,
    pub accepted_count: usize,
    /// The 20 newest open pending titles, newest first. Retained for existing
    /// JSON consumers; the markdown renders the operator sections instead.
    pub pending_titles: Vec<String>,
    /// The 20 newest open accepted titles, newest first.
    pub accepted_titles: Vec<String>,
}

/// One open handoff as the briefing shows it.
#[derive(Debug, Clone, Serialize)]
pub struct BriefItem {
    pub id: String,
    /// First 8 characters of the UUID — what agents actually quote on the bus.
    pub short_id: String,
    pub title: String,
    pub from_agent: String,
    pub to_agent: Option<String>,
    pub status: String,
    pub priority: String,
    /// Whole days between `created_at` and the briefing's `now`.
    pub age_days: i64,
}

/// Open action handoffs addressed to the operator. Oldest first.
#[derive(Debug, Serialize)]
pub struct OperatorQueue {
    pub operator: String,
    /// True total, independent of the cap below.
    pub total: usize,
    pub truncated: bool,
    /// Has this slug ever appeared on a handoff at all? An empty queue is good
    /// news; an empty queue because the slug is misspelled is the briefing
    /// lying about the one thing it exists to report. Only meaningful when
    /// `total == 0`.
    pub operator_seen: bool,
    pub items: Vec<BriefItem>,
}

#[derive(Debug, Serialize)]
pub struct StuckGroup {
    pub to_agent: String,
    pub items: Vec<BriefItem>,
}

/// Open handoffs somebody other than the operator owes, past the age at which
/// "in progress" stops being a credible reading.
#[derive(Debug, Serialize)]
pub struct StuckSection {
    pub pending_after_days: i64,
    pub accepted_after_days: i64,
    /// True total across every group, independent of `STUCK_SECTION_CAP`.
    pub total: usize,
    pub truncated: bool,
    pub groups: Vec<StuckGroup>,
}

#[derive(Debug, Serialize)]
pub struct RecipientCount {
    pub to_agent: String,
    pub open: usize,
}

/// A bucket the briefing reports as a count rather than a list.
#[derive(Debug, Serialize)]
pub struct BucketCount {
    pub count: usize,
    pub oldest_age_days: Option<i64>,
    /// Machine findings only: how many carry `repeat_count > 0`, i.e. are
    /// still firing rather than filed once and forgotten.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub repeating: Option<usize>,
}

/// The tail of the open set: shape only, no titles.
#[derive(Debug, Serialize)]
pub struct CountsSection {
    /// Every open action handoff bucketed by recipient. Sums to `open_count`.
    pub by_recipient: Vec<RecipientCount>,
    pub self_addressed: BucketCount,
    pub machine_findings: BucketCount,
}

/// The three operator sections, derived purely from a set of open handoffs and
/// a fixed `now`.
#[derive(Debug, Serialize)]
pub struct OperatorSections {
    pub waiting_on_you: OperatorQueue,
    pub stuck: StuckSection,
    pub counts: CountsSection,
}

/// Stateless fleet-wide briefing generation for the REST delivery endpoint.
pub async fn generate_briefing_inner(
    pool: &PgPool,
    briefing_type: &str,
    operator: &str,
) -> Result<serde_json::Value, String> {
    let is_weekly = briefing_type == "weekly";

    // Real totals — not `len()` of a bounded page.
    let counts = crate::repo::handoff_repo::count_open_handoffs(pool)
        .await
        .map_err(|e| format!("Failed to count handoffs: {e}"))?;

    // The whole open action set, projected. Briefings show actionable work
    // only — notify-class FYIs are not "pending" in any meaningful sense.
    let open = crate::repo::handoff_repo::list_open_action_briefs(pool, operator)
        .await
        .map_err(|e| format!("Failed to list handoffs: {e}"))?;

    // An empty operator queue has two very different causes: nothing needs the
    // human, or the caller named a slug no handoff has ever used. Both render
    // as silence otherwise, and silence is exactly what this section exists to
    // break. `validate_agent_name` only checks shape — "CC-Stelth" is a
    // perfectly valid slug that matches nothing. Only pay for the lookup when
    // the queue is actually empty; on DB error assume seen rather than cry wolf.
    let operator_seen = if open.iter().any(|r| r.is_operator) {
        true
    } else {
        crate::repo::handoff_repo::agent_seen_before(pool, operator, uuid::Uuid::nil())
            .await
            .unwrap_or(true)
    };

    let now = Utc::now();
    let sections = triage(&open, operator, operator_seen, &now);

    let handoff_data = HandoffSummaryData {
        open_count: counts.open as usize,
        pending_count: counts.pending as usize,
        accepted_count: counts.accepted as usize,
        pending_titles: legacy_titles(&open, "pending"),
        accepted_titles: legacy_titles(&open, "accepted"),
    };

    let md = build_markdown(is_weekly, &now, &handoff_data, &sections);

    let result = BriefingData {
        briefing_type: briefing_type.to_string(),
        generated_at: now.format("%Y-%m-%d %H:%M UTC").to_string(),
        operator: operator.to_string(),
        handoffs: handoff_data,
        waiting_on_you: sections.waiting_on_you,
        stuck: sections.stuck,
        counts: sections.counts,
        content: md,
    };

    serde_json::to_value(&result).map_err(|e| format!("Failed to serialize briefing: {e}"))
}

/// Whole days elapsed. Clamped at zero so a row created a hair in the future
/// (clock skew between producers) reads as "today" rather than negative.
fn age_days(now: &DateTime<Utc>, created: &DateTime<Utc>) -> i64 {
    (*now - *created).num_days().max(0)
}

fn to_item(row: &OpenActionBrief, now: &DateTime<Utc>) -> BriefItem {
    let id = row.id.to_string();
    BriefItem {
        short_id: id.chars().take(8).collect(),
        id,
        title: row.title.clone(),
        from_agent: row.from_agent.clone(),
        to_agent: row.to_agent.clone(),
        status: row.status.clone(),
        priority: row.priority.clone(),
        age_days: age_days(now, &row.created_at),
    }
}

/// Is this row past the age at which its status stops being credible?
fn is_stuck(status: &str, age: i64) -> bool {
    match status {
        "pending" => age > STUCK_PENDING_DAYS,
        "accepted" => age > STUCK_ACCEPTED_DAYS,
        _ => false,
    }
}

/// The 20 newest open titles of one status, newest first — the pre-operator-view
/// shape, kept so existing JSON consumers keep working. `rows` is oldest first.
fn legacy_titles(rows: &[OpenActionBrief], status: &str) -> Vec<String> {
    rows.iter()
        .rev()
        .filter(|r| r.status == status)
        .take(LEGACY_TITLE_LIMIT)
        .map(|r| r.title.clone())
        .collect()
}

/// Split the open set into the three operator sections. Pure: every age comes
/// from the injected `now`, so the whole shape is testable against a fixture.
///
/// `rows` is expected oldest first (as the repo query returns it); ordering
/// within every section follows from that.
fn triage(
    rows: &[OpenActionBrief],
    operator: &str,
    operator_seen: bool,
    now: &DateTime<Utc>,
) -> OperatorSections {
    // ── 1. Waiting on you ──
    // Filtered on `is_operator` alone: a machine finding or a self-addressed
    // row that names the operator is still work only the human can clear, so
    // it is titled here even though those classes are counts-only elsewhere.
    let mine: Vec<&OpenActionBrief> = rows.iter().filter(|r| r.is_operator).collect();
    let waiting_on_you = OperatorQueue {
        operator: operator.to_string(),
        total: mine.len(),
        truncated: mine.len() > OPERATOR_SECTION_CAP,
        operator_seen,
        items: mine
            .iter()
            .take(OPERATOR_SECTION_CAP)
            .map(|r| to_item(r, now))
            .collect(),
    };

    // ── 2. Stuck ──
    // Somebody else's queue, genuinely addressed to another agent, filed by a
    // human-facing agent rather than a monitor, and old enough that "in
    // progress" no longer explains it.
    let mut groups: Vec<StuckGroup> = Vec::new();
    let mut stuck_total = 0usize;
    let mut stuck_shown = 0usize;
    for row in rows {
        if row.is_operator || row.is_self_addressed || row.origin == "machine" {
            continue;
        }
        let item = to_item(row, now);
        if !is_stuck(&row.status, item.age_days) {
            continue;
        }
        // Count every stuck row; render only up to the cap. Counting first is
        // what keeps `total` a total rather than a page length.
        stuck_total += 1;
        if stuck_shown >= STUCK_SECTION_CAP {
            continue;
        }
        stuck_shown += 1;
        let key = row.to_agent.as_deref().unwrap_or(UNADDRESSED);
        // Case-insensitive, matching how the SQL compares agents: `CC-Cloud`
        // and `cc-cloud` are one agent and must not split into two groups.
        // First spelling seen wins as the display name.
        match groups
            .iter_mut()
            .find(|g| g.to_agent.eq_ignore_ascii_case(key))
        {
            // `rows` is oldest first, so appending keeps each group oldest first.
            Some(g) => g.items.push(item),
            None => groups.push(StuckGroup {
                to_agent: key.to_string(),
                items: vec![item],
            }),
        }
    }
    // Groups appear in order of their oldest item — which is already the order
    // they were created in above — so no re-sort is needed.
    let stuck = StuckSection {
        pending_after_days: STUCK_PENDING_DAYS,
        accepted_after_days: STUCK_ACCEPTED_DAYS,
        total: stuck_total,
        truncated: stuck_total > stuck_shown,
        groups,
    };

    // ── 3. Everything else, as counts ──
    let mut by_recipient: Vec<RecipientCount> = Vec::new();
    let mut self_addressed = BucketCount {
        count: 0,
        oldest_age_days: None,
        repeating: None,
    };
    let mut machine = BucketCount {
        count: 0,
        oldest_age_days: None,
        repeating: Some(0),
    };
    for row in rows {
        let key = row.to_agent.as_deref().unwrap_or(UNADDRESSED);
        match by_recipient
            .iter_mut()
            .find(|c| c.to_agent.eq_ignore_ascii_case(key))
        {
            Some(c) => c.open += 1,
            None => by_recipient.push(RecipientCount {
                to_agent: key.to_string(),
                open: 1,
            }),
        }
        // `rows` is oldest first, so the first row into a bucket is its oldest.
        let age = age_days(now, &row.created_at);
        if row.is_self_addressed {
            self_addressed.count += 1;
            self_addressed.oldest_age_days.get_or_insert(age);
        }
        if row.origin == "machine" {
            machine.count += 1;
            machine.oldest_age_days.get_or_insert(age);
            if row.repeat_count > 0 {
                machine.repeating = machine.repeating.map(|n| n + 1);
            }
        }
    }
    // Busiest queue first; name breaks ties so the line is stable run to run.
    by_recipient.sort_by(|a, b| {
        b.open
            .cmp(&a.open)
            .then_with(|| a.to_agent.cmp(&b.to_agent))
    });

    OperatorSections {
        waiting_on_you,
        stuck,
        counts: CountsSection {
            by_recipient,
            self_addressed,
            machine_findings: machine,
        },
    }
}

fn age_label(days: i64) -> String {
    if days == 0 {
        "today".to_string()
    } else {
        format!("{days}d")
    }
}

/// One item line. Leading fields are joined rather than concatenated so an
/// omitted qualifier (a normal-priority item) leaves no dangling separator.
fn item_line(lead: &str, qualifiers: &[&str], item: &BriefItem) -> String {
    let mut parts: Vec<&str> = vec![lead];
    parts.extend_from_slice(qualifiers);
    format!(
        "- {} · {} — from {} · {}\n",
        parts.join(" · "),
        item.title,
        item.from_agent,
        item.short_id
    )
}

/// Qualifier list for an item: priority, but only when it says something.
fn priority_qualifier(item: &BriefItem) -> Option<&str> {
    (item.priority != "normal").then_some(item.priority.as_str())
}

fn build_markdown(
    is_weekly: bool,
    now: &DateTime<Utc>,
    handoffs: &HandoffSummaryData,
    sections: &OperatorSections,
) -> String {
    let mut md = String::new();

    md.push_str(&format!(
        "# {} Operational Briefing\n",
        if is_weekly { "Weekly" } else { "Daily" }
    ));
    md.push_str(&format!(
        "*Generated: {}*\n\n",
        now.format("%Y-%m-%d %H:%M UTC")
    ));
    md.push_str(&format!(
        "**{} open handoff(s) fleet-wide** ({} pending, {} accepted)\n\n",
        handoffs.open_count, handoffs.pending_count, handoffs.accepted_count
    ));

    // ── 1. Waiting on you ──
    // Always rendered, even when empty: "nothing is waiting on you" and "the
    // section failed to render" must not look the same.
    let q = &sections.waiting_on_you;
    md.push_str(&format!("## Waiting on you ({})\n\n", q.operator));
    if q.items.is_empty() {
        if q.operator_seen {
            md.push_str("Nothing is waiting on you.\n\n");
        } else {
            // Do not report a misrouted briefing as a clear queue.
            md.push_str(&format!(
                "No handoff has ever named `{}` — check the operator slug.\n\n",
                q.operator
            ));
        }
    } else {
        if q.truncated {
            md.push_str(&format!(
                "Showing the {} oldest of {} — the rest are in the counts below.\n\n",
                q.items.len(),
                q.total
            ));
        }
        for item in &q.items {
            let quals: Vec<&str> = priority_qualifier(item).into_iter().collect();
            md.push_str(&item_line(&age_label(item.age_days), &quals, item));
        }
        md.push('\n');
    }

    // ── 2. Stuck ──
    let s = &sections.stuck;
    md.push_str(&format!(
        "## Stuck (pending open > {}d, accepted open > {}d)\n\n",
        s.pending_after_days, s.accepted_after_days
    ));
    if s.groups.is_empty() {
        md.push_str("Nothing else is open past those thresholds.\n\n");
    } else {
        if s.truncated {
            let shown: usize = s.groups.iter().map(|g| g.items.len()).sum();
            md.push_str(&format!(
                "Showing the {shown} oldest of {} — the rest are in the counts below.\n\n",
                s.total
            ));
        }
        for group in &s.groups {
            md.push_str(&format!("**{}**\n", group.to_agent));
            for item in &group.items {
                let mut quals: Vec<&str> = vec![item.status.as_str()];
                quals.extend(priority_qualifier(item));
                // "open Nd", not "accepted Nd ago": there is no accepted_at,
                // so the only honest claim is how long the row has been open.
                md.push_str(&item_line(
                    &format!("open {}d", item.age_days),
                    &quals,
                    item,
                ));
            }
            md.push('\n');
        }
    }

    // ── 3. Everything else, counts only ──
    let c = &sections.counts;
    // Not "everything else": `by_recipient` partitions the *whole* open set,
    // including rows already titled above, and the two buckets overlap it. The
    // heading has to match the arithmetic or the operator can't trust it.
    md.push_str("## Open set at a glance\n\n");
    let by_recipient = if c.by_recipient.is_empty() {
        "none".to_string()
    } else {
        c.by_recipient
            .iter()
            .map(|r| format!("{} {}", r.to_agent, r.open))
            .collect::<Vec<_>>()
            .join(", ")
    };
    md.push_str(&format!("- Open by recipient: {by_recipient}\n"));
    md.push_str(&format!(
        "- Self-addressed claims: {}\n",
        bucket_phrase(&c.self_addressed, "open")
    ));
    md.push_str(&format!(
        "- Machine findings: {}\n",
        bucket_phrase(&c.machine_findings, "open")
    ));

    md
}

/// "6 open (2 still firing), oldest 12d" — or "none" when the bucket is empty,
/// so the line never renders a misleading "0, oldest -".
fn bucket_phrase(bucket: &BucketCount, noun: &str) -> String {
    if bucket.count == 0 {
        return "none".to_string();
    }
    let mut s = format!("{} {noun}", bucket.count);
    // Only when it says something — "(0 still firing)" is noise.
    if let Some(repeating) = bucket.repeating.filter(|n| *n > 0) {
        s.push_str(&format!(" ({repeating} still firing)"));
    }
    if let Some(age) = bucket.oldest_age_days {
        s.push_str(&format!(", oldest {}", age_label(age)));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    fn ts(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn now() -> DateTime<Utc> {
        ts("2026-09-19T12:00:00Z")
    }

    #[allow(clippy::too_many_arguments)]
    fn row(
        id: &str,
        from: &str,
        to: Option<&str>,
        status: &str,
        priority: &str,
        title: &str,
        origin: &str,
        repeat_count: i32,
        created: &str,
        operator: &str,
    ) -> OpenActionBrief {
        let to_agent = to.map(str::to_string);
        OpenActionBrief {
            id: Uuid::parse_str(id).unwrap(),
            from_agent: from.to_string(),
            to_agent: to_agent.clone(),
            status: status.to_string(),
            priority: priority.to_string(),
            title: title.to_string(),
            origin: origin.to_string(),
            repeat_count,
            created_at: ts(created),
            // Mirrors what the SQL projection computes.
            is_operator: to_agent
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(operator)),
            is_self_addressed: to_agent
                .as_deref()
                .is_some_and(|t| t.eq_ignore_ascii_case(from)),
        }
    }

    /// Oldest first, as the repo query returns it.
    fn fixture(operator: &str) -> Vec<OpenActionBrief> {
        vec![
            row(
                "019e0d79-3a7f-7902-86cc-db4a573c1071",
                "CC-Stealth",
                Some(operator),
                "pending",
                "high",
                "Blocked: need the IT budget figure",
                "agent",
                0,
                "2026-08-24T12:00:00Z",
                operator,
            ),
            row(
                "019a1111-0000-7000-8000-000000000001",
                "CC-Cloud",
                Some("CC-Cloud"),
                "accepted",
                "normal",
                "Claiming the certificate rotation",
                "agent",
                0,
                "2026-08-30T12:00:00Z",
                operator,
            ),
            row(
                "019b2222-0000-7000-8000-000000000002",
                "Monitor",
                Some("CC-Cloud"),
                "pending",
                "normal",
                "[auto] disk above 80% on the media volume",
                "machine",
                3,
                "2026-09-05T12:00:00Z",
                operator,
            ),
            row(
                "019c3333-0000-7000-8000-000000000003",
                "CC-Stealth",
                Some("CC-Cloud"),
                "accepted",
                "normal",
                "Deploy the briefing change",
                "agent",
                0,
                "2026-09-08T12:00:00Z",
                operator,
            ),
            row(
                "019d4444-0000-7000-8000-000000000004",
                "Codex-HSR",
                None,
                "pending",
                "low",
                "Anyone up for reviewing the adapter tests",
                "agent",
                0,
                "2026-09-15T12:00:00Z",
                operator,
            ),
            row(
                "019e5555-0000-7000-8000-000000000005",
                "CC-Cloud",
                Some(operator),
                "pending",
                "normal",
                "Approve the switch decommission",
                "agent",
                0,
                "2026-09-16T12:00:00Z",
                operator,
            ),
            // Fresh: below the pending threshold, must not reach Stuck.
            row(
                "019f6666-0000-7000-8000-000000000006",
                "CC-Stealth",
                Some("Codex-HSR"),
                "pending",
                "normal",
                "Fresh request nobody is late on",
                "agent",
                0,
                "2026-09-18T12:00:00Z",
                operator,
            ),
        ]
    }

    fn summary() -> HandoffSummaryData {
        HandoffSummaryData {
            open_count: 7,
            pending_count: 5,
            accepted_count: 2,
            pending_titles: vec![],
            accepted_titles: vec![],
        }
    }

    #[test]
    fn waiting_on_you_is_oldest_first_and_operator_scoped() {
        let s = triage(&fixture("Operator"), "Operator", true, &now());
        let titles: Vec<&str> = s
            .waiting_on_you
            .items
            .iter()
            .map(|i| i.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec![
                "Blocked: need the IT budget figure",
                "Approve the switch decommission"
            ]
        );
        assert_eq!(s.waiting_on_you.items[0].age_days, 26);
        assert_eq!(s.waiting_on_you.items[1].age_days, 3);
        assert_eq!(s.waiting_on_you.total, 2);
        assert!(!s.waiting_on_you.truncated);
    }

    #[test]
    fn a_custom_operator_slug_reshapes_the_section() {
        let s = triage(&fixture("Ops-Lead"), "Ops-Lead", true, &now());
        assert_eq!(s.waiting_on_you.operator, "Ops-Lead");
        assert_eq!(s.waiting_on_you.total, 2);
    }

    #[test]
    fn stuck_excludes_operator_self_addressed_machine_and_fresh_rows() {
        let s = triage(&fixture("Operator"), "Operator", true, &now());
        let titles: Vec<&str> = s
            .stuck
            .groups
            .iter()
            .flat_map(|g| g.items.iter())
            .map(|i| i.title.as_str())
            .collect();
        assert_eq!(
            titles,
            vec![
                "Deploy the briefing change",
                "Anyone up for reviewing the adapter tests"
            ]
        );
        assert_eq!(s.stuck.total, 2);
        assert_eq!(s.stuck.groups[0].to_agent, "CC-Cloud");
        assert_eq!(s.stuck.groups[1].to_agent, UNADDRESSED);
    }

    /// Exactly at the threshold is not yet stuck; one day past it is.
    #[test]
    fn stuck_thresholds_are_strictly_greater_than() {
        assert!(!is_stuck("pending", STUCK_PENDING_DAYS));
        assert!(is_stuck("pending", STUCK_PENDING_DAYS + 1));
        assert!(!is_stuck("accepted", STUCK_ACCEPTED_DAYS));
        assert!(is_stuck("accepted", STUCK_ACCEPTED_DAYS + 1));
        // A pending row old enough to be stuck would not be if it were accepted.
        assert!(!is_stuck("accepted", STUCK_PENDING_DAYS + 1));
    }

    #[test]
    fn counts_cover_every_open_row_exactly_once() {
        let s = triage(&fixture("Operator"), "Operator", true, &now());
        let total: usize = s.counts.by_recipient.iter().map(|r| r.open).sum();
        assert_eq!(total, 7, "by_recipient must partition the whole open set");
        assert_eq!(s.counts.self_addressed.count, 1);
        assert_eq!(s.counts.self_addressed.oldest_age_days, Some(20));
        assert_eq!(s.counts.machine_findings.count, 1);
        assert_eq!(s.counts.machine_findings.repeating, Some(1));
        assert_eq!(s.counts.machine_findings.oldest_age_days, Some(14));
    }

    #[test]
    fn operator_section_caps_but_reports_the_true_total() {
        let mut rows = Vec::new();
        for i in 0..(OPERATOR_SECTION_CAP + 5) {
            rows.push(row(
                &format!("019e0d79-3a7f-7902-86cc-{:012x}", i),
                "CC-Stealth",
                Some("Operator"),
                "pending",
                "normal",
                "piled up",
                "agent",
                0,
                "2026-09-01T12:00:00Z",
                "Operator",
            ));
        }
        let s = triage(&rows, "Operator", true, &now());
        assert_eq!(s.waiting_on_you.items.len(), OPERATOR_SECTION_CAP);
        assert_eq!(s.waiting_on_you.total, OPERATOR_SECTION_CAP + 5);
        assert!(s.waiting_on_you.truncated);
        let md = build_markdown(false, &now(), &summary(), &s);
        assert!(md.contains(&format!("of {}", OPERATOR_SECTION_CAP + 5)));
    }

    #[test]
    fn markdown_renders_all_three_sections() {
        let s = triage(&fixture("Operator"), "Operator", true, &now());
        let md = build_markdown(false, &now(), &summary(), &s);
        let expected = "\
# Daily Operational Briefing
*Generated: 2026-09-19 12:00 UTC*

**7 open handoff(s) fleet-wide** (5 pending, 2 accepted)

## Waiting on you (Operator)

- 26d · high · Blocked: need the IT budget figure — from CC-Stealth · 019e0d79
- 3d · Approve the switch decommission — from CC-Cloud · 019e5555

## Stuck (pending open > 3d, accepted open > 7d)

**CC-Cloud**
- open 11d · accepted · Deploy the briefing change — from CC-Stealth · 019c3333

**(unaddressed)**
- open 4d · pending · low · Anyone up for reviewing the adapter tests — from Codex-HSR · 019d4444

## Open set at a glance

- Open by recipient: CC-Cloud 3, Operator 2, (unaddressed) 1, Codex-HSR 1
- Self-addressed claims: 1 open, oldest 20d
- Machine findings: 1 open (1 still firing), oldest 14d
";
        assert_eq!(md, expected);
    }

    #[test]
    fn empty_sections_say_so_rather_than_vanishing() {
        let s = triage(&[], "Operator", true, &now());
        let md = build_markdown(
            false,
            &now(),
            &HandoffSummaryData {
                open_count: 0,
                pending_count: 0,
                accepted_count: 0,
                pending_titles: vec![],
                accepted_titles: vec![],
            },
            &s,
        );
        assert!(md.contains("## Waiting on you (Operator)"));
        assert!(md.contains("Nothing is waiting on you."));
        assert!(md.contains("## Stuck"));
        assert!(md.contains("Nothing else is open past those thresholds."));
        assert!(md.contains("- Open by recipient: none"));
        assert!(md.contains("- Self-addressed claims: none"));
        assert!(md.contains("- Machine findings: none"));
    }

    #[test]
    fn weekly_only_changes_the_title() {
        let s = triage(&fixture("Operator"), "Operator", true, &now());
        let md = build_markdown(true, &now(), &summary(), &s);
        assert!(md.starts_with("# Weekly Operational Briefing\n"));
    }

    #[test]
    fn legacy_title_lists_stay_newest_first() {
        let rows = fixture("Operator");
        let pending = legacy_titles(&rows, "pending");
        assert_eq!(pending[0], "Fresh request nobody is late on");
        assert_eq!(
            pending.last().unwrap(),
            "Blocked: need the IT budget figure"
        );
        assert_eq!(
            legacy_titles(&rows, "accepted"),
            vec![
                "Deploy the briefing change",
                "Claiming the certificate rotation"
            ]
        );
    }

    #[test]
    fn future_dated_rows_read_as_today_not_negative() {
        assert_eq!(age_days(&now(), &ts("2026-09-20T12:00:00Z")), 0);
        assert_eq!(age_label(0), "today");
    }

    /// An empty queue for a slug nothing has ever used is a misrouted briefing,
    /// not good news, and must not render as "nothing is waiting on you".
    #[test]
    fn an_unknown_operator_slug_is_flagged_not_reported_as_a_clear_queue() {
        let s = triage(&[], "CC-Stelth", false, &now());
        let md = build_markdown(false, &now(), &summary(), &s);
        assert!(
            md.contains("No handoff has ever named `CC-Stelth` — check the operator slug."),
            "got {md}"
        );
        assert!(!md.contains("Nothing is waiting on you."));
        assert!(!s.waiting_on_you.operator_seen);
    }

    /// The whole premise is that work piles up, so the stuck render is capped
    /// too — but the count stays true.
    #[test]
    fn stuck_render_is_capped_while_the_total_stays_true() {
        let over = STUCK_SECTION_CAP + 7;
        let rows: Vec<OpenActionBrief> = (0..over)
            .map(|i| {
                row(
                    &format!("019c3333-0000-7000-8000-{i:012x}"),
                    "CC-Stealth",
                    Some("CC-Cloud"),
                    "pending",
                    "normal",
                    "piled up",
                    "agent",
                    0,
                    "2026-09-01T12:00:00Z",
                    "Operator",
                )
            })
            .collect();
        let s = triage(&rows, "Operator", true, &now());
        let shown: usize = s.stuck.groups.iter().map(|g| g.items.len()).sum();
        assert_eq!(shown, STUCK_SECTION_CAP);
        assert_eq!(s.stuck.total, over);
        assert!(s.stuck.truncated);
        let md = build_markdown(false, &now(), &summary(), &s);
        assert!(md.contains(&format!("Showing the {STUCK_SECTION_CAP} oldest of {over}")));
    }

    /// The SQL compares agents case-insensitively; the Rust grouping must too,
    /// or one agent becomes two groups and two count lines.
    #[test]
    fn grouping_folds_case_and_keeps_the_first_spelling() {
        let rows = vec![
            row(
                "019c3333-0000-7000-8000-000000000001",
                "CC-Stealth",
                Some("CC-Cloud"),
                "pending",
                "normal",
                "first spelling",
                "agent",
                0,
                "2026-09-01T12:00:00Z",
                "Operator",
            ),
            row(
                "019c3333-0000-7000-8000-000000000002",
                "CC-Stealth",
                Some("cc-cloud"),
                "pending",
                "normal",
                "lowercase sibling",
                "agent",
                0,
                "2026-09-02T12:00:00Z",
                "Operator",
            ),
        ];
        let s = triage(&rows, "Operator", true, &now());
        assert_eq!(s.stuck.groups.len(), 1, "one agent, one group");
        assert_eq!(
            s.stuck.groups[0].to_agent, "CC-Cloud",
            "first spelling wins"
        );
        assert_eq!(s.stuck.groups[0].items.len(), 2);
        assert_eq!(s.counts.by_recipient.len(), 1, "one agent, one count line");
        assert_eq!(s.counts.by_recipient[0].open, 2);
    }

    /// Machine and self-addressed rows are counts-only *elsewhere*. Addressed
    /// to the operator they are work only the human can clear, so they are
    /// titled. Pinned because the docs describe the exclusion loosely.
    #[test]
    fn operator_addressed_machine_and_self_rows_are_still_titled() {
        let rows = vec![
            row(
                "019c3333-0000-7000-8000-000000000001",
                "Monitor",
                Some("Operator"),
                "pending",
                "high",
                "[auto] certificate expires in 3 days",
                "machine",
                2,
                "2026-09-01T12:00:00Z",
                "Operator",
            ),
            row(
                "019c3333-0000-7000-8000-000000000002",
                "Operator",
                Some("Operator"),
                "pending",
                "normal",
                "note to self",
                "agent",
                0,
                "2026-09-02T12:00:00Z",
                "Operator",
            ),
        ];
        let s = triage(&rows, "Operator", true, &now());
        assert_eq!(s.waiting_on_you.total, 2);
        let md = build_markdown(false, &now(), &summary(), &s);
        assert!(md.contains("[auto] certificate expires in 3 days"));
        assert!(md.contains("note to self"));
        // …and they are still counted in their buckets.
        assert_eq!(s.counts.machine_findings.count, 1);
        assert_eq!(s.counts.self_addressed.count, 1);
    }

    #[test]
    fn a_bucket_with_nothing_repeating_omits_the_clause() {
        let bucket = BucketCount {
            count: 3,
            oldest_age_days: Some(5),
            repeating: Some(0),
        };
        assert_eq!(bucket_phrase(&bucket, "open"), "3 open, oldest 5d");
    }
}
