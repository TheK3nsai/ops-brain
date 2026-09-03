pub mod briefings;
pub mod check_in;
pub mod coordination;
mod helpers;
pub mod knowledge;
pub mod live;
pub mod search;
mod shared;

use rmcp::{
    handler::server::wrapper::Parameters, model::*, tool, tool_handler, tool_router,
    ErrorData as McpError, ServerHandler,
};
use sqlx::PgPool;

use crate::embeddings::EmbeddingClient;
use crate::live::LiveHub;

#[derive(Clone)]
pub struct OpsBrain {
    pub(crate) pool: PgPool,
    pub(crate) embedding_client: Option<EmbeddingClient>,
    pub(crate) live_hub: LiveHub,
}

#[tool_router]
impl OpsBrain {
    pub fn new(pool: PgPool, embedding_client: Option<EmbeddingClient>) -> Self {
        Self {
            pool,
            embedding_client,
            live_hub: LiveHub::default(),
        }
    }

    pub fn with_live_hub(
        pool: PgPool,
        embedding_client: Option<EmbeddingClient>,
        live_hub: LiveHub,
    ) -> Self {
        Self {
            pool,
            embedding_client,
            live_hub,
        }
    }

    // ===== KNOWLEDGE TOOLS =====

    #[tool(
        name = "add_knowledge",
        description = "Add a knowledge base entry (lesson, gotcha, tip). Requires author (your agent name).",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn add_knowledge(
        &self,
        params: Parameters<knowledge::AddKnowledgeParams>,
        ext: Extensions,
    ) -> Result<CallToolResult, McpError> {
        let bound = helpers::bound_agent(&ext);
        Ok(knowledge::handle_add_knowledge(self, params.0, bound.as_deref()).await)
    }

    #[tool(
        name = "update_knowledge",
        description = "Update an existing knowledge base entry by ID. Only provided fields are updated.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn update_knowledge(
        &self,
        params: Parameters<knowledge::UpdateKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_update_knowledge(self, params.0).await)
    }

    #[tool(
        name = "delete_knowledge",
        description = "Delete a knowledge base entry by ID.",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn delete_knowledge(
        &self,
        params: Parameters<knowledge::DeleteKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_delete_knowledge(self, params.0).await)
    }

    #[tool(
        name = "search_bus",
        description = "Search knowledge and/or handoffs. \
        Set tables param for multi-table. Modes: fts/semantic/hybrid (default). \
        Empty query or '*' browses recent entries. Responses report the effective \
        limit, whether it was clamped, and whether more results exist per table.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn search_bus(
        &self,
        params: Parameters<knowledge::SearchKnowledgeParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(knowledge::handle_search_knowledge(self, params.0).await)
    }

    // ===== HANDOFF TOOLS =====

    #[tool(
        name = "create_handoff",
        description = "Create a handoff task for another agent/session to continue.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
    )]
    async fn create_handoff(
        &self,
        params: Parameters<coordination::CreateHandoffParams>,
        ext: Extensions,
    ) -> Result<CallToolResult, McpError> {
        let bound = helpers::bound_agent(&ext);
        Ok(coordination::handle_create_handoff(self, params.0, bound.as_deref()).await)
    }

    #[tool(
        name = "get_handoff",
        description = "Get one handoff by its full UUID, including its complete body and context.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn get_handoff(
        &self,
        params: Parameters<coordination::GetHandoffParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_get_handoff(self, params.0).await)
    }

    #[tool(
        name = "accept_handoff",
        description = "Accept a pending handoff, marking it as accepted by you",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
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
        `merged` when the bundle reaches main.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = false
        )
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
        Optional ISO-8601 `since` filters by reply timestamp. Responses report the \
        effective limit, whether it was clamped, and whether more replies exist.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_replies_to_me(
        &self,
        params: Parameters<coordination::ListRepliesToMeParams>,
        ext: Extensions,
    ) -> Result<CallToolResult, McpError> {
        let bound = helpers::bound_agent(&ext);
        Ok(coordination::handle_list_replies_to_me(self, params.0, bound.as_deref()).await)
    }

    #[tool(
        name = "mark_merged",
        description = "Flip a handoff to status=merged and record the merge commit. \
        Typically called by an integrator script after the bundle containing the \
        handoff's commit_hash lands in main. Idempotent on identical merge_commit; \
        refuses to overwrite a different one.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn mark_merged(
        &self,
        params: Parameters<coordination::MarkMergedParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_mark_merged(self, params.0).await)
    }

    #[tool(
        name = "list_handoffs",
        description = "List handoffs with optional filters. Use status='pending' to see what needs attention. \
        Responses report the effective limit, whether it was clamped, and whether more handoffs exist.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_handoffs(
        &self,
        params: Parameters<coordination::ListHandoffsParams>,
    ) -> Result<CallToolResult, McpError> {
        Ok(coordination::handle_list_handoffs(self, params.0).await)
    }

    #[tool(
        name = "delete_handoff",
        description = "Permanently delete a handoff by ID (hard delete)",
        annotations(
            read_only_hint = false,
            destructive_hint = true,
            idempotent_hint = true,
            open_world_hint = false
        )
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
        slug — e.g. 'CC-Stealth', 'Codex-HSR').",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn check_in(
        &self,
        params: Parameters<check_in::CheckInParams>,
        ext: Extensions,
    ) -> Result<CallToolResult, McpError> {
        let bound = helpers::bound_agent(&ext);
        Ok(check_in::handle_check_in(self, params.0, bound.as_deref()).await)
    }

    // ===== LIVE PEERS: ephemeral online-only transport =====

    #[tool(
        name = "list_live_peers",
        description = "List currently connected live adapters. Online-only; absence means use a handoff.",
        annotations(
            read_only_hint = true,
            destructive_hint = false,
            idempotent_hint = true,
            open_world_hint = false
        )
    )]
    async fn list_live_peers(
        &self,
        _params: Parameters<live::ListLivePeersParams>,
        ext: Extensions,
    ) -> Result<CallToolResult, McpError> {
        let bound = helpers::bound_agent(&ext);
        Ok(live::handle_list_live_peers(self, bound.as_deref()).await)
    }

    #[tool(
        name = "send_live_message",
        description = "Send untrusted text to one connected peer. Best-effort; never queues offline.",
        annotations(
            read_only_hint = false,
            destructive_hint = false,
            idempotent_hint = false,
            open_world_hint = true
        )
    )]
    async fn send_live_message(
        &self,
        params: Parameters<live::SendLiveMessageParams>,
        ext: Extensions,
    ) -> Result<CallToolResult, McpError> {
        let bound = helpers::bound_agent(&ext);
        Ok(live::handle_send_live_message(self, params.0, bound.as_deref()).await)
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
                 the rest of the team: handoffs, cross-agent knowledge, and \
                 online live peers. Live messages are best-effort; use a handoff \
                 whenever the target peer is absent. \
                 Identify yourself with a free-form `agent_name` (slug, \
                 e.g. 'CC-Stealth', 'Codex-HSR'). One deployment is one trusted \
                 coordination domain; scoped knowledge queries withhold unsafe \
                 cross-client content until acknowledge_cross_client=true.",
            )
    }
}

#[cfg(test)]
mod tests {
    use super::helpers::*;
    use super::OpsBrain;
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

    #[test]
    fn live_surface_is_exactly_fifteen_tools() {
        let tools = OpsBrain::tool_router().list_all();
        let mut names: Vec<String> = tools.iter().map(|tool| tool.name.to_string()).collect();
        names.sort();
        assert_eq!(
            names,
            vec![
                "accept_handoff",
                "add_knowledge",
                "check_in",
                "complete_handoff",
                "create_handoff",
                "delete_handoff",
                "delete_knowledge",
                "get_handoff",
                "list_handoffs",
                "list_live_peers",
                "list_replies_to_me",
                "mark_merged",
                "search_bus",
                "send_live_message",
                "update_knowledge",
            ]
        );
    }

    #[test]
    fn tool_annotations_distinguish_reads_and_destructive_writes() {
        let tools = OpsBrain::tool_router().list_all();
        let search = tools.iter().find(|tool| tool.name == "search_bus").unwrap();
        let delete = tools
            .iter()
            .find(|tool| tool.name == "delete_handoff")
            .unwrap();
        assert_eq!(
            search.annotations.as_ref().unwrap().read_only_hint,
            Some(true)
        );
        assert_eq!(
            delete.annotations.as_ref().unwrap().destructive_hint,
            Some(true)
        );
    }

    #[test]
    fn json_results_include_structured_content() {
        let result = json_result(&serde_json::json!({"ok": true}));
        assert_eq!(
            result.structured_content,
            Some(serde_json::json!({"ok": true}))
        );
        assert_eq!(result.is_error, Some(false));
    }

    #[test]
    fn json_results_wrap_non_object_values() {
        let result = json_result(&vec!["first", "second"]);
        assert_eq!(
            result.structured_content,
            Some(serde_json::json!({"result": ["first", "second"]}))
        );
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

    // ===== generic cross-client gating tests =====

    #[test]
    fn filter_other_entity_cross_client_withheld() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), false)];

        let result = filter_cross_client(items, "shared_item", Some(beta_id), false, &lookup);

        assert!(result.allowed.is_empty());
        assert_eq!(result.withheld_notices.len(), 1);
        assert_eq!(result.withheld_notices[0]["entity_type"], "shared_item");
        assert_eq!(result.withheld_notices[0]["owning_client_slug"], "alpha");
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "withheld");
    }

    #[test]
    fn filter_other_entity_cross_client_safe_allowed() {
        let (alpha_id, beta_id, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), true)];

        let result = filter_cross_client(items, "shared_item", Some(beta_id), false, &lookup);

        assert_eq!(result.allowed.len(), 1);
        assert!(result.withheld_notices.is_empty());
        assert_eq!(result.audit_entries.len(), 1);
        assert_eq!(result.audit_entries[0].2, "released_safe");
    }

    #[test]
    fn filter_other_entity_same_client_allowed() {
        let (alpha_id, _, lookup) = make_lookup();
        let item_id = Uuid::now_v7();
        let items = vec![make_item(item_id, Some(alpha_id), false)];

        let result = filter_cross_client(items, "shared_item", Some(alpha_id), false, &lookup);

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
}
