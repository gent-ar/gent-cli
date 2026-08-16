//! Durable, provider-neutral values for a persisted agent-chat user prompt.

use serde::{Deserialize, Serialize};

use crate::{
    AgentChatConversationId, AgentChatRequestId, AgentChatRunId, ConversationMessage, HostEpoch,
    Receipt, ReceiptId,
};

/// The request intent determines scheduling only; neither variant starts a provider here.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatPromptDisposition {
    Send,
    Queue,
}

/// Truthful local delivery state for an accepted prompt.
///
/// `gentd` may acknowledge durable storage before it has authority to launch a provider. This
/// value makes that boundary visible without exposing a provider session or claiming execution.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub enum AgentChatPromptDelivery {
    /// The caller explicitly requested local queueing; no provider delivery was attempted.
    Queued,
    /// The prompt is durable but awaits a separately authorized provider lifecycle.
    AwaitingProvider,
}

/// Durable input for one user prompt whose identities are derived from request correlation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatPromptCreate {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub disposition: AgentChatPromptDisposition,
    pub text: String,
}

/// One settled receipt and immutable user message assigned to the resolved current run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatPromptSaved {
    pub receipt: Receipt,
    pub run_id: AgentChatRunId,
    pub message: ConversationMessage,
    pub disposition: AgentChatPromptDisposition,
    pub delivery: AgentChatPromptDelivery,
}

impl AgentChatPromptDisposition {
    /// Derives delivery without launching or inspecting a provider.
    #[must_use]
    pub const fn delivery(self) -> AgentChatPromptDelivery {
        match self {
            Self::Send => AgentChatPromptDelivery::AwaitingProvider,
            Self::Queue => AgentChatPromptDelivery::Queued,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{AgentChatPromptDelivery, AgentChatPromptDisposition};

    #[test]
    fn prompt_disposition_is_closed_and_provider_free() {
        assert!(serde_json::from_str::<AgentChatPromptDisposition>("\"send\"").is_ok());
        assert!(serde_json::from_str::<AgentChatPromptDisposition>("\"later\"").is_err());
    }

    #[test]
    fn delivery_is_derived_without_claiming_a_provider_launch() {
        assert_eq!(
            AgentChatPromptDisposition::Send.delivery(),
            AgentChatPromptDelivery::AwaitingProvider
        );
        assert_eq!(
            AgentChatPromptDisposition::Queue.delivery(),
            AgentChatPromptDelivery::Queued
        );
    }
}
