use gent_types::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, PermissionDecisionRequest,
    PermissionDecisionResponse, Receipt,
};
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_PERMISSIONS_CAPABILITY: &str = "agent-chat-permissions-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum AgentChatPermissionFrame {
    PendingRead {
        request_id: AgentChatRequestId,
        conversation_id: AgentChatConversationId,
        run_id: AgentChatRunId,
    },
    Pending {
        request_id: AgentChatRequestId,
        request: Option<PermissionDecisionRequest>,
    },
    Respond {
        request_id: AgentChatRequestId,
        receipt_id: gent_types::ReceiptId,
        response: PermissionDecisionResponse,
    },
    Accepted {
        request_id: AgentChatRequestId,
        receipt: Receipt,
        decision_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{AGENT_CHAT_PERMISSIONS_CAPABILITY, AgentChatPermissionFrame};
    use gent_types::{AgentChatConversationId, AgentChatRequestId, AgentChatRunId};

    #[test]
    fn pending_read_is_run_scoped_and_versioned() {
        let frame = AgentChatPermissionFrame::PendingRead {
            request_id: AgentChatRequestId("request-1".into()),
            conversation_id: AgentChatConversationId("conversation-1".into()),
            run_id: AgentChatRunId("run-1".into()),
        };
        let value = serde_json::to_value(frame).unwrap();
        assert_eq!(
            AGENT_CHAT_PERMISSIONS_CAPABILITY,
            "agent-chat-permissions-v1"
        );
        assert_eq!(value["type"], "pendingRead");
        assert!(value.get("providerSessionId").is_none());
    }
}
