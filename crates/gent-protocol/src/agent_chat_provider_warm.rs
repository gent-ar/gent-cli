use gent_types::{AgentChatConversationId, AgentChatRequestId, AgentChatRunId};
use serde::{Deserialize, Serialize};

pub const AGENT_CHAT_PROVIDER_WARM_CAPABILITY: &str = "agent-chat-provider-warm-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatProviderWarm {
    pub request_id: AgentChatRequestId,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderWarmOutcome {
    Warming,
    AlreadyWarm,
    TurnActive,
    Superseded,
    UnsupportedProvider,
    PoolFull,
    Unavailable,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AgentChatProviderWarmResult {
    pub request_id: AgentChatRequestId,
    pub conversation_id: AgentChatConversationId,
    pub run_id: AgentChatRunId,
    pub outcome: ProviderWarmOutcome,
}

impl AgentChatProviderWarm {
    #[must_use]
    pub fn result(&self, outcome: ProviderWarmOutcome) -> AgentChatProviderWarmResult {
        AgentChatProviderWarmResult {
            request_id: self.request_id.clone(),
            conversation_id: self.conversation_id.clone(),
            run_id: self.run_id.clone(),
            outcome,
        }
    }
}
