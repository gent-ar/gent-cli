use gent_types::{AgentChatSession, AgentChatSessionId};
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_SESSIONS_CAPABILITY: &str = "agent-chat-sessions-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", content = "body", rename_all = "camelCase")]
pub enum AgentChatSessionFrame {
    CreateRequest {
        request_id: String,
        session: AgentChatSession,
    },
    Created {
        request_id: String,
        session: AgentChatSession,
    },
    ListRequest {
        request_id: String,
        workspace_id: String,
    },
    List {
        request_id: String,
        workspace_id: String,
        sessions: Vec<AgentChatSession>,
    },
    SelectRequest {
        request_id: String,
        session_id: AgentChatSessionId,
    },
    Selected {
        request_id: String,
        session: AgentChatSession,
    },
    AttachRequest {
        request_id: String,
        session_id: AgentChatSessionId,
        conversation_id: String,
    },
    Attached {
        request_id: String,
        session: AgentChatSession,
    },
}
