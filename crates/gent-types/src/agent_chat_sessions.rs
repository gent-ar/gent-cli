use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(transparent)]
pub struct AgentChatSessionId(pub String);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatSession {
    pub session_id: AgentChatSessionId,
    pub workspace_id: String,
    pub name: String,
    pub conversation_ids: Vec<String>,
    pub created_at: i64,
    pub updated_at: i64,
}

impl AgentChatSession {
    pub fn validate(&self) -> Result<(), &'static str> {
        if !valid_id(&self.session_id.0)
            || !valid_id(&self.workspace_id)
            || !valid_text(&self.name, 256)
            || self.updated_at < self.created_at
            || self.conversation_ids.iter().any(|id| !valid_id(id))
        {
            return Err("agent chat session metadata is invalid");
        }
        Ok(())
    }
}

fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_text(value: &str, max: usize) -> bool {
    !value.trim().is_empty() && value.len() <= max && !value.contains('\0')
}
