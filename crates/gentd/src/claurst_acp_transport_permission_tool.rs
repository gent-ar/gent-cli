use serde_json::Value;

use super::ClaurstAcpTransport;

pub(crate) struct PermissionTool {
    pub(crate) tool_use_id: String,
    pub(crate) tool_name: String,
    pub(crate) input: Option<Value>,
}

impl<S> ClaurstAcpTransport<S> {
    pub(crate) fn claim_permission_tool(&mut self, permission: &Value) -> Option<PermissionTool> {
        let id = permission.get("toolCallId")?.as_str()?;
        let title = permission.get("title")?.as_str()?;
        let input = permission.get("rawInput")?.as_object()?;
        if id.is_empty()
            || id.len() > 256
            || id.chars().any(char::is_control)
            || title.is_empty()
            || title.len() > 256
            || !title
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        {
            return None;
        }
        let name = title.to_owned();
        let input = Value::Object(input.clone());
        if self.permission_claimed_ids.len() >= 4096 || self.permission_claimed_ids.contains(id) {
            return None;
        }
        if let Some(tool) = self.open_tools.iter_mut().find(|tool| tool.id == id) {
            if tool.permission_claimed
                || tool.authority_name.as_deref() != Some(name.as_str())
                || tool.input.as_ref().is_some_and(|known| known != &input)
            {
                return None;
            }
            tool.permission_claimed = true;
        }
        self.permission_claimed_ids.insert(id.into());
        Some(PermissionTool {
            tool_use_id: id.into(),
            tool_name: name,
            input: Some(input),
        })
    }
}
