use serde::Serialize;
use sqlx::PgPool;

/// Structured briefing data returned alongside the markdown content.
#[derive(Debug, Serialize)]
pub struct BriefingData {
    pub briefing_type: String,
    pub generated_at: String,
    pub handoffs: HandoffSummaryData,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct HandoffSummaryData {
    pub open_count: usize,
    pub pending_count: usize,
    pub accepted_count: usize,
    pub pending_titles: Vec<String>,
    pub accepted_titles: Vec<String>,
}

/// Stateless fleet-wide briefing generation for the REST delivery endpoint.
pub async fn generate_briefing_inner(
    pool: &PgPool,
    briefing_type: &str,
) -> Result<serde_json::Value, String> {
    let is_weekly = briefing_type == "weekly";

    // Real totals — not `len()` of a bounded page. The title lists below come
    // from a LIMIT-20 window and are labeled "recent" when they undercount.
    let counts = crate::repo::handoff_repo::count_open_handoffs(pool)
        .await
        .map_err(|e| format!("Failed to count handoffs: {e}"))?;

    // ── Pending handoffs (title lists) ──
    // Briefings show actionable work only — notify-class FYIs are not "pending"
    // in any meaningful sense.
    let open_handoffs =
        crate::repo::handoff_repo::list_open_handoffs(pool, None, None, None, false, 20)
            .await
            .map_err(|e| format!("Failed to list handoffs: {e}"))?;

    let pending_titles: Vec<String> = open_handoffs
        .iter()
        .filter(|h| h.status == "pending")
        .map(|h| h.title.clone())
        .collect();
    let accepted_titles: Vec<String> = open_handoffs
        .iter()
        .filter(|h| h.status == "accepted")
        .map(|h| h.title.clone())
        .collect();

    let handoff_data = HandoffSummaryData {
        open_count: counts.open as usize,
        pending_count: counts.pending as usize,
        accepted_count: counts.accepted as usize,
        pending_titles,
        accepted_titles,
    };

    // ── Build markdown ──
    let now = chrono::Utc::now();
    let md = build_markdown(is_weekly, &now, &handoff_data);

    let result = BriefingData {
        briefing_type: briefing_type.to_string(),
        generated_at: now.format("%Y-%m-%d %H:%M UTC").to_string(),
        handoffs: handoff_data,
        content: md,
    };

    serde_json::to_value(&result).map_err(|e| format!("Failed to serialize briefing: {e}"))
}

fn build_markdown(
    is_weekly: bool,
    now: &chrono::DateTime<chrono::Utc>,
    handoffs: &HandoffSummaryData,
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

    // Handoffs (fleet-wide)
    md.push_str("## Handoffs (fleet-wide)\n\n");
    if handoffs.open_count == 0 {
        md.push_str("No open handoffs.\n\n");
    } else {
        md.push_str(&format!(
            "**{} open handoff(s)** ({} pending, {} accepted)\n\n",
            handoffs.open_count, handoffs.pending_count, handoffs.accepted_count
        ));
        push_title_list(
            &mut md,
            "Pending",
            handoffs.pending_count,
            &handoffs.pending_titles,
        );
        push_title_list(
            &mut md,
            "Accepted",
            handoffs.accepted_count,
            &handoffs.accepted_titles,
        );
        md.push('\n');
    }

    md
}

/// Append a labeled title list. When `total` exceeds the shown titles (the
/// list is a bounded page), the header says "recent" and spells out the gap so
/// the count and the list can't silently disagree.
fn push_title_list(md: &mut String, label: &str, total: usize, titles: &[String]) {
    if titles.is_empty() {
        return;
    }
    if total > titles.len() {
        md.push_str(&format!(
            "{label} (showing {} most recent of {total}):\n",
            titles.len()
        ));
    } else {
        md.push_str(&format!("{label}:\n"));
    }
    for title in titles {
        md.push_str(&format!("- {title}\n"));
    }
}
