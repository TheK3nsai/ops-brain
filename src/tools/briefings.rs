use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use super::helpers::{error_result, json_result, not_found_with_suggestions};
use rmcp::model::*;

#[derive(Debug, Deserialize, JsonSchema)]
pub struct GenerateBriefingParams {
    /// Briefing type: "daily" or "weekly"
    pub briefing_type: String,
    /// Client slug to scope the briefing to a specific client (optional — omit for global briefing)
    pub client_slug: Option<String>,
}

/// Structured briefing data returned alongside the markdown content.
#[derive(Debug, Serialize)]
pub struct BriefingData {
    pub briefing_type: String,
    pub client: Option<String>,
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

// ===== HANDLERS =====

pub(crate) async fn handle_generate_briefing(
    brain: &super::OpsBrain,
    p: GenerateBriefingParams,
) -> CallToolResult {
    if let Err(msg) = crate::validation::validate_required(
        &p.briefing_type,
        "briefing_type",
        crate::validation::BRIEFING_TYPES,
    ) {
        return error_result(&msg);
    }

    let client = match &p.client_slug {
        Some(slug) => match crate::repo::client_repo::get_client_by_slug(&brain.pool, slug).await {
            Ok(Some(c)) => Some(c),
            Ok(None) => return not_found_with_suggestions(&brain.pool, "Client", slug).await,
            Err(e) => return error_result(&format!("Database error: {e}")),
        },
        None => None,
    };

    match crate::api::generate_briefing_inner(
        &brain.pool,
        &p.briefing_type.to_lowercase(),
        client.as_ref(),
    )
    .await
    {
        Ok(output) => json_result(&output),
        Err(e) => error_result(&e),
    }
}
