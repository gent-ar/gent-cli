use std::sync::Arc;

use async_trait::async_trait;
use claurst_core::types::ToolDefinition;
use claurst_tools::{PermissionLevel, Tool, ToolContext, ToolResult};

pub struct McpTool {
    name: String,
    routed_name: String,
    definition: ToolDefinition,
    manager: Arc<claurst_mcp::McpManager>,
}

fn bindable(part: &str) -> bool {
    !part.is_empty()
        && part
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.'))
}

pub fn exposed_name(server: &str, routed_name: &str) -> Option<String> {
    let tool = routed_name.strip_prefix(server)?.strip_prefix('_')?;
    (bindable(server) && !server.contains("__") && bindable(tool))
        .then(|| format!("mcp__{server}__{tool}"))
}

pub fn tools(manager: &Arc<claurst_mcp::McpManager>) -> Vec<Box<dyn Tool>> {
    manager
        .all_tool_definitions()
        .into_iter()
        .filter_map(|(server, definition)| {
            let name = exposed_name(&server, &definition.name)?;
            Some(Box::new(McpTool {
                name,
                routed_name: definition.name.clone(),
                definition,
                manager: manager.clone(),
            }) as Box<dyn Tool>)
        })
        .collect()
}

#[async_trait]
impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }

    fn description(&self) -> &str {
        &self.definition.description
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    fn input_schema(&self) -> serde_json::Value {
        self.definition.input_schema.clone()
    }

    async fn execute(&self, input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let args = (!input.is_null()).then_some(input);
        match self.manager.call_tool(&self.routed_name, args).await {
            Ok(result) if result.is_error => ToolResult::error(claurst_mcp::mcp_result_to_string(&result)),
            Ok(result) => ToolResult::success(claurst_mcp::mcp_result_to_string(&result)),
            Err(error) => ToolResult::error(format!("MCP tool '{}' failed: {error}", self.name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::exposed_name;

    #[test]
    fn exposed_names_are_gent_bindable() {
        assert_eq!(
            exposed_name("gent-chat", "gent-chat_send_message").as_deref(),
            Some("mcp__gent-chat__send_message")
        );
        assert_eq!(exposed_name("a__b", "a__b_tool"), None);
        assert_eq!(exposed_name("srv", "srv_bad name"), None);
        assert_eq!(exposed_name("srv", "other_tool"), None);
    }
}
