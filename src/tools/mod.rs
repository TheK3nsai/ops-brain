pub mod briefings;
pub mod check_in;
pub mod coordination;
mod helpers;
pub mod knowledge;
mod search;
mod shared;
mod zammad;

use rmcp::{
    handler::server::wrapper::Parameters, model::*, tool, tool_handler, tool_router,
    ErrorData as McpError, ServerHandler,
};
use sqlx::PgPool;

use crate::embeddings::EmbeddingClient;
use crate::zammad::ZammadConfig;

#[derive(Clone)]
pub struct OpsBrain {
    pub(crate) pool: PgPool,
    pub(crate) embedding_client: Option<EmbeddingClient>,
    pub(crate) zammad_config: Option<ZammadConfig>,
}

#[tool_router]
impl OpsBrain {
    pub fn new(
        pool: PgPool,
        embedding_client: Option<EmbeddingClient>,
        zammad_config: Option<ZammadConfig>,
    ) -> Self {
        Self {
            pool,
            embedding_client,
            zammad_config,
        }
    }

    // ===== KNOWLEDGE TOOLS =====

    #[tool(
        name = "add_knowledge",
        description = "Add a knowledge base entry (lesson, gotcha, tip). Requires author (your agent name)."
    )]
    async fn add_knowledge(
        &self,
        params: Parameters<knowledge::AddKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_add_knowledge(self, params.0).await)
    }

    #[tool(
        name = "update_knowledge",
        description = "Update an existing knowledge base entry by ID. Only provided fields are updated."
    )]
    async fn update_knowledge(
        &self,
        params: Parameters<knowledge::UpdateKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_update_knowledge(self, params.0).await)
    }

    #[tool(
        name = "delete_knowledge",
        description = "Delete a knowledge base entry by ID."
    )]
    async fn delete_knowledge(
        &self,
        params: Parameters<knowledge::DeleteKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_delete_knowledge(self, params.0).await)
    }

    #[tool(
        name = "search_knowledge",
        description = "Search knowledge and/or handoffs. \
        Set tables param for multi-table. Modes: fts/semantic/hybrid (default). \
        Empty query or '*' browses recent entries."
    )]
    async fn search_knowledge(
        &self,
        params: Parameters<knowledge::SearchKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_search_knowledge(self, params.0).await)
    }

    #[tool(
        name = "list_knowledge",
        description = "List knowledge base entries, optionally filtered by category or client"
    )]
    async fn list_knowledge(
        &self,
        params: Parameters<knowledge::ListKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_list_knowledge(self, params.0).await)
    }

    // ===== HANDOFF TOOLS =====

    #[tool(
        name = "create_handoff",
        description = "Create a handoff task for another agent/session to continue."
    )]
    async fn create_handoff(
        &self,
        params: Parameters<coordination::CreateHandoffParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_create_handoff(self, params.0).await)
    }

    #[tool(
        name = "accept_handoff",
        description = "Accept a pending handoff, marking it as accepted by you"
    )]
    async fn accept_handoff(
        &self,
        params: Parameters<coordination::UpdateHandoffStatusParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_accept_handoff(self, params.0).await)
    }

    #[tool(
        name = "complete_handoff",
        description = "Mark a handoff as completed. Optional `commit_hash` records the work \
        ref (typically a git SHA) so `mark_merged` can later flip the same handoff to \
        `merged` when the bundle reaches main."
    )]
    async fn complete_handoff(
        &self,
        params: Parameters<coordination::CompleteHandoffParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_complete_handoff(self, params.0).await)
    }

    #[tool(
        name = "list_replies_to_me",
        description = "List handoffs that reply to ones you sent. Returns handoffs whose \
        `in_reply_to` references a handoff with your `agent_name` as `from_agent`. \
        Optional ISO-8601 `since` filters by reply timestamp."
    )]
    async fn list_replies_to_me(
        &self,
        params: Parameters<coordination::ListRepliesToMeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_list_replies_to_me(self, params.0).await)
    }

    #[tool(
        name = "mark_merged",
        description = "Flip a handoff to status=merged and record the merge commit. \
        Typically called by an integrator script after the bundle containing the \
        handoff's commit_hash lands in main. Idempotent on identical merge_commit; \
        refuses to overwrite a different one."
    )]
    async fn mark_merged(
        &self,
        params: Parameters<coordination::MarkMergedParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_mark_merged(self, params.0).await)
    }

    #[tool(
        name = "list_handoffs",
        description = "List handoffs with optional filters. Use status='pending' to see what needs attention."
    )]
    async fn list_handoffs(
        &self,
        params: Parameters<coordination::ListHandoffsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_list_handoffs(self, params.0).await)
    }

    #[tool(
        name = "search_handoffs",
        description = "Search handoff titles and bodies. Modes: fts (default), semantic, or hybrid (RRF)."
    )]
    async fn search_handoffs(
        &self,
        params: Parameters<coordination::SearchHandoffsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_search_handoffs(self, params.0).await)
    }

    #[tool(
        name = "delete_handoff",
        description = "Permanently delete a handoff by ID (hard delete)"
    )]
    async fn delete_handoff(
        &self,
        params: Parameters<coordination::DeleteHandoffParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_delete_handoff(self, params.0).await)
    }

    // ===== TEAM BUS: pending-work query =====

    #[tool(
        name = "check_in",
        description = "Pending-work query: open action handoffs addressed to you and recent \
        notify-class handoffs (compact). Pass `agent_name` (your free-form agent \
        slug — e.g. 'CC-Stealth', 'codex-hsr')."
    )]
    async fn check_in(
        &self,
        params: Parameters<check_in::CheckInParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(check_in::handle_check_in(self, params.0).await)
    }

    // ===== SEMANTIC SEARCH TOOLS =====

    #[tool(
        name = "backfill_embeddings",
        description = "Generate missing embeddings for records. Use after setup or when API key was unavailable."
    )]
    async fn backfill_embeddings(
        &self,
        params: Parameters<search::BackfillEmbeddingsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(search::handle_backfill_embeddings(self, params.0).await)
    }

    // ===== ZAMMAD TICKET TOOLS =====

    #[tool(
        name = "list_tickets",
        description = "List Zammad tickets. Filter by client, state, priority. Omit client_slug for all clients."
    )]
    async fn list_tickets(
        &self,
        params: Parameters<zammad::ListTicketsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_list_tickets(self, params.0).await)
    }

    #[tool(
        name = "get_ticket",
        description = "Get a Zammad ticket by ID with full article history (messages, notes, time accounting)."
    )]
    async fn get_ticket(
        &self,
        params: Parameters<zammad::GetTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_get_ticket(self, params.0).await)
    }

    #[tool(
        name = "create_ticket",
        description = "Create a Zammad ticket. Resolves client_slug to group/org/customer."
    )]
    async fn create_ticket(
        &self,
        params: Parameters<zammad::CreateTicketParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_create_ticket(self, params.0).await)
    }

    #[tool(
        name = "search_tickets",
        description = "Search Zammad tickets via Elasticsearch syntax."
    )]
    async fn search_tickets(
        &self,
        params: Parameters<zammad::SearchTicketsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(zammad::handle_search_tickets(self, params.0).await)
    }

    // ===== BRIEFING TOOLS =====

    #[tool(
        name = "generate_briefing",
        description = "Generate a daily/weekly operational briefing. Aggregates pending \
        handoffs and Zammad tickets. Optionally client-scoped. Stored for history."
    )]
    async fn generate_briefing(
        &self,
        params: Parameters<briefings::GenerateBriefingParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(briefings::handle_generate_briefing(self, params.0).await)
    }
}

#[tool_handler]
impl ServerHandler for OpsBrain {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("ops-brain", env!("CARGO_PKG_VERSION")))
            .with_instructions(
                "ops-brain is the team bus. Your local instructions, filesystem, and git \
                 history are the source of truth — reach for ops-brain only when you need \
                 the rest of the team: handoffs, cross-agent knowledge, briefings, and \
                 Zammad tickets. Identify yourself with a free-form `agent_name` (slug, \
                 e.g. 'CC-Stealth', 'codex-hsr'). Default-deny across clients: \
                 cross-client content requires acknowledge_cross_client=true.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::helpers::*;
    use std::collections::HashMap;
    use uuid::Uuid;

    fn make_item(id: Uuid, client_id: Option<Uuid>, cross_client_safe: bool) -> serde_json::Value {
        let mut obj = serde_json::json!({
            "id": id.to_string(),
            "title": "Test Item",
            "cross_client_safe": cross_client_safe,
        });
        if let Some(cid) = client_id {
            obj["client_id"] = serde_json::Value::String(cid.to_string());
        }
        obj
    }

    fn make_lookup() -> (Uuid, Uuid, HashMap<Uuid, (String, String)>) {
        let alpha_id = Uuid::now_v7();
        let beta_id = Uuid::now_v7();
        let mut lookup = HashMap::new();
        lookup.insert(alpha_id, ("alpha".to_string(), "Alpha Corp".to_string()));
        lookup.insert(beta_id, ("beta".to_string(), "Beta Inc".to_string()));
        (alpha_id, beta_id, lookup)
    }

    // ===== filter_cross_client tests =====

    #[test]
    fn filter_no_requesting_client_allows_all() {
        let (alpha_id, _, lookup) = make_lookup();
        let items = vec![
            make_item(Uuid::now_v7(), Some(alpha_id), false),
            make_item(Uuid::now_v7(), None, false),
        ];

        let result = filter_cross_client(items, "knowledge", None, false, &lookup);

        assert_eq!(result.allowed.len(), 2);
        assert!(result.withheld_notices.is_empty());
        assert!(result.audit_entries.is_empty());
    }

    #[test]
    fn filter_global_content_always_allowed() {
        let (alpha_id, _, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, None, false)];

        let result = filter_cross_client(items, "knowledge", Some(alpha_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert!(result.audit_entries.is_empty());
        assert_eq!(result.allowed[0]["_client_name"], "Global");
        assert!(result.allowed[0]["_client_slug"].is_null());
    }

    #[test]
    fn filter_same_client_allowed() {
        let (alpha_id, _, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), false)];

        let result = filter_cross_client(items, "knowledge", Some(alpha_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert!(result.audit_entries.is_empty());
        assert_eq!(result.allowed[0]["_client_slug"], "alpha");
        assert_eq!(result.allowed[0]["_client_name"], "Alpha Corp");
    }

    #[test]
    fn filter_cross_client_safe_allowed() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), true)];

        let result = filter_cross_client(items, "knowledge", Some(beta_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].0, item_id);
        assert_eq!(result.audit_entries[0].1, Some(alpha_id));
        assert_eq!(result.audit_entries[0].2, "released_safe");
    }

    #[test]
    fn filter_cross_client_acknowledged_released() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), false)];

        let result = filter_cross_client(items, "knowledge", Some(beta_id), true, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "released");
    }

    #[test]
    fn filter_cross_client_withheld() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), false)];

        let result = filter_cross_client(items, "knowledge", Some(beta_id), false, &lookup);

        assert!(result.allowed.is_empty());
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["count"], 1);
        assert_eq!(result.withheld_notices[0]["owning_client_slug"], "alpha");
        assert_eq!(result.withheld_notices[0]["entity_type"], "knowledge");
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "withheld");
    }

    #[test]
    fn filter_multiple_withheld_grouped_by_client() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let items = vec![
            make_item(Uuid::now_v7(), Some(alpha_id), false),
            make_item(Uuid::now_v7(), Some(alpha_id), false),
        ];

        let result = filter_cross_client(items, "knowledge", Some(beta_id), false, &lookup);

        assert!(result.allowed.is_empty());
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["count"], 2);
        assert_eq!(result.audit_entries.len(), 2);
    }

    #[test]
    fn filter_mixed_items() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let items = vec![
            make_item(Uuid::now_v7(), None, false), // global → allowed
            make_item(Uuid::now_v7(), Some(beta_id), false), // same client → allowed
            make_item(Uuid::now_v7(), Some(alpha_id), true), // diff client, safe → allowed
            make_item(Uuid::now_v7(), Some(alpha_id), false), // diff client, not safe → withheld
        ];

        let result = filter_cross_client(items, "knowledge", Some(beta_id), false, &lookup);

        assert_eq!(result.allowed.len(), 3);
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["count"], 1);
        assert_eq!(result.audit_entries.len(), 2);
    }

    // ===== incident cross-client gating tests =====

    #[test]
    fn filter_incident_cross_client_withheld() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), false)];

        let result = filter_cross_client(items, "incident", Some(beta_id), false, &lookup);

        assert!(result.allowed.is_empty());
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["entity_type"], "incident");
        assert_eq!(result.withheld_notices[0]["owning_client_slug"], "alpha");
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "withheld");
    }

    #[test]
    fn filter_incident_cross_client_safe_allowed() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), true)];

        let result = filter_cross_client(items, "incident", Some(beta_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "released_safe");
    }

    #[test]
    fn filter_incident_same_client_allowed() {
        let (alpha_id, _, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), false)];

        let result = filter_cross_client(items, "incident", Some(alpha_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert!(result.audit_entries.is_empty());
    }

    // ===== inject_provenance tests =====

    #[test]
    fn provenance_with_client() {
        let (alpha_id, _, lookup) = make_lookup();
        let mut item = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
            "client_id": alpha_id.to_string(),
        });

        inject_provenance(&mut item, &lookup);

        assert_eq!(item["_client_slug"], "alpha");
        assert_eq!(item["_client_name"], "Alpha Corp");
    }

    #[test]
    fn provenance_without_client() {
        let (_, _, lookup) = make_lookup();
        let mut item = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
        });

        inject_provenance(&mut item, &lookup);

        assert!(item["_client_slug"].is_null());
        assert_eq!(item["_client_name"], "Global");
    }

    #[test]
    fn provenance_unknown_client() {
        let lookup = HashMap::new();
        let unknown_id = Uuid::now_v7();
        let mut item = serde_json::json!({
            "id": Uuid::now_v7().to_string(),
            "client_id": unknown_id.to_string(),
        });

        inject_provenance(&mut item, &lookup);

        assert!(item.get("_client_slug").is_none());
    }

    // ===== compact mode tests =====
}
