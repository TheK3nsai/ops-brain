//! Modern transport dispatch reuses the exact existing tool stubs, including
//! their identity and visibility guards.
use super::*;

impl OpsBrain {
    pub(crate) fn modern_tool_catalog() -> Vec<rmcp::model::Tool> {
        Self::tool_router().list_all()
    }
    pub(crate) async fn call_modern_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
        ext: Extensions,
    ) -> Result<CallToolResult, McpError> {
        macro_rules! dispatch {
            ($($method:ident),+ $(,)?) => {
                match name {
                    $(stringify!($method) => self.$method(Parameters(
                        serde_json::from_value(arguments).map_err(|_| {
                            McpError::invalid_params("invalid tool arguments", None)
                        })?
                    ), ext).await,)+
                    _ => Err(McpError::new(ErrorCode::METHOD_NOT_FOUND, "Tool not found", None)),
                }
            };
        }
        dispatch!(
            add_knowledge,
            update_knowledge,
            delete_knowledge,
            search_bus,
            create_handoff,
            get_handoff,
            accept_handoff,
            complete_handoff,
            list_handoffs,
            delete_handoff,
            list_replies_to_me,
            mark_merged,
            check_in
        )
    }
}
