use gent_types::{AgentChatConversationId, AgentChatRequestId, AgentChatRunId, ReceiptId};
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_STEER_ENQUEUE_CAPABILITY: &str = "agent-chat-steer-enqueue-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatSteerPrompt {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub conversation_id: AgentChatConversationId,
    pub expected_run_id: AgentChatRunId,
    pub expected_turn_id: String,
    pub text: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    #[serde(default)]
    pub tool_source_ids: Vec<String>,
}
