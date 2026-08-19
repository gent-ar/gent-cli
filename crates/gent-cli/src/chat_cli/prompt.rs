//! Exact durable prompt correlation for the terminal client.

use std::path::PathBuf;

use gent_protocol::AgentChatIntentFrame;
use gent_types::{AgentChatConversationId, AgentChatPromptDelivery, AgentChatRunId, Receipt};

/// Exact durable identities created by one accepted terminal prompt.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PromptAccepted {
    pub(crate) conversation_id: AgentChatConversationId,
    pub(crate) run_id: AgentChatRunId,
    pub(crate) turn_id: String,
    pub(crate) delivery: AgentChatPromptDelivery,
    pub(crate) receipt: Receipt,
}

/// Persists one interactive terminal prompt without starting a provider process.
pub(crate) async fn send(
    data_dir: Option<PathBuf>,
    no_autostart: bool,
    conversation_id: String,
    text: String,
) -> Result<PromptAccepted, Box<dyn std::error::Error>> {
    let response = super::exchange(
        data_dir,
        no_autostart,
        AgentChatIntentFrame::SendPrompt {
            request_id: super::request_id(None),
            receipt_id: super::receipt_id(None),
            conversation_id: AgentChatConversationId(conversation_id),
            text,
        },
    )
    .await?;
    let AgentChatIntentFrame::Accepted {
        conversation_id,
        run_id,
        turn_id,
        delivery,
        receipt,
        ..
    } = response
    else {
        return Err("daemon did not accept the agent-chat prompt".into());
    };
    Ok(PromptAccepted {
        conversation_id,
        run_id,
        turn_id,
        delivery,
        receipt,
    })
}
