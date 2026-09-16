//! Capability-gated agent-chat intent transport.
//!
//! This adapter owns JSON framing and response correlation only. It does not create a
//! conversation, persist a prompt, or launch a provider; those authority decisions stay behind
//! [`RuntimeApi`].

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY, AgentChatIntentFrame,
    conversation_links::{CONVERSATION_LINKS_CAPABILITY, is_conversation_link_frame},
    write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{
    agent_chat_intent_error::AgentChatIntentError, api::RuntimeApi, transport::write_error,
};

#[path = "agent_chat_transport_replies.rs"]
mod replies;
use replies::validate_replies;

pub(crate) trait IntentPort {
    fn exchange(
        &self,
        request: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError>;
}

impl<R: RuntimeApi> IntentPort for R {
    fn exchange(
        &self,
        request: AgentChatIntentFrame,
    ) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError> {
        self.agent_chat_intent(request)
    }
}

/// Dispatches one finite, receipt-backed agent-chat exchange after capability negotiation.
pub(crate) async fn dispatch<S, R>(
    stream: &mut S,
    runtime: &R,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    if let Some(frame) = waiting_frame(capabilities, raw) {
        return wait_off_thread(stream, runtime, frame).await;
    }
    dispatch_port(stream, runtime, capabilities, raw).await
}

fn waiting_frame(capabilities: &CapabilitySet, raw: &Value) -> Option<AgentChatIntentFrame> {
    if !supports(capabilities) || !links_enabled(capabilities) {
        return None;
    }
    serde_json::from_value::<AgentChatIntentFrame>(raw.clone())
        .ok()
        .filter(|frame| matches!(frame, AgentChatIntentFrame::WaitForConversations { .. }))
}

async fn wait_off_thread<S, R>(
    stream: &mut S,
    runtime: &R,
    request: AgentChatIntentFrame,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    R: RuntimeApi,
{
    let runtime = runtime.clone();
    let waited = request.clone();
    let result = tokio::task::spawn_blocking(move || runtime.agent_chat_intent(waited)).await?;
    match result {
        Ok(replies) => match validate_replies(&request, &replies) {
            Ok(()) => {
                for reply in replies {
                    write_json_frame(stream, &reply).await?;
                }
            }
            Err(message) => write_error(stream, "invalidAgentChatResponse", message).await?,
        },
        Err(error) => write_error(stream, error.code, &error.message).await?,
    }
    Ok(true)
}

fn links_enabled(capabilities: &CapabilitySet) -> bool {
    capabilities
        .0
        .iter()
        .any(|capability| capability == CONVERSATION_LINKS_CAPABILITY)
}

pub(crate) async fn dispatch_port<S, P>(
    stream: &mut S,
    port: &P,
    capabilities: &CapabilitySet,
    raw: &Value,
) -> Result<bool, Box<dyn std::error::Error + Send + Sync>>
where
    S: AsyncWrite + Unpin,
    P: IntentPort,
{
    if !supports(capabilities) {
        return Ok(false);
    }
    let Ok(request) = serde_json::from_value::<AgentChatIntentFrame>(raw.clone()) else {
        return Ok(false);
    };
    if matches!(request, AgentChatIntentFrame::ImportTranscript { .. })
        && !capabilities
            .0
            .iter()
            .any(|capability| capability == AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY)
    {
        return Ok(false);
    }
    if is_conversation_link_frame(&request) && !links_enabled(capabilities) {
        return Ok(false);
    }
    if !matches!(request, AgentChatIntentFrame::ImportTranscript { .. })
        && !is_conversation_link_frame(&request)
        && !capabilities
            .0
            .iter()
            .any(|capability| capability == AGENT_CHAT_INTENTS_CAPABILITY)
    {
        return Ok(false);
    }
    if !is_client_request(&request) {
        write_error(
            stream,
            "invalidAgentChatIntent",
            "agent chat response frames are server-only",
        )
        .await?;
        return Ok(true);
    }
    match port.exchange(request.clone()) {
        Ok(replies) => match validate_replies(&request, &replies) {
            Ok(()) => {
                for reply in replies {
                    write_json_frame(stream, &reply).await?;
                }
            }
            Err(message) => write_error(stream, "invalidAgentChatResponse", message).await?,
        },
        Err(error) => write_error(stream, error.code, &error.message).await?,
    }
    Ok(true)
}

fn supports(capabilities: &CapabilitySet) -> bool {
    capabilities.0.iter().any(|capability| {
        capability == AGENT_CHAT_INTENTS_CAPABILITY
            || capability == AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY
            || capability == CONVERSATION_LINKS_CAPABILITY
    })
}

fn is_client_request(frame: &AgentChatIntentFrame) -> bool {
    matches!(
        frame,
        AgentChatIntentFrame::CreateConversation { .. }
            | AgentChatIntentFrame::ImportTranscript { .. }
            | AgentChatIntentFrame::SendPrompt { .. }
            | AgentChatIntentFrame::QueuePrompt { .. }
            | AgentChatIntentFrame::SendPromptWithTools { .. }
            | AgentChatIntentFrame::QueuePromptWithTools { .. }
            | AgentChatIntentFrame::CancelQueuedPrompt { .. }
            | AgentChatIntentFrame::SteerQueuedPrompt { .. }
            | AgentChatIntentFrame::ContinueFromSavedHistory { .. }
            | AgentChatIntentFrame::SwitchSelection { .. }
            | AgentChatIntentFrame::Interrupt { .. }
            | AgentChatIntentFrame::Decision { .. }
            | AgentChatIntentFrame::Subscribe { .. }
            | AgentChatIntentFrame::ForkConversation { .. }
            | AgentChatIntentFrame::CreateLinkedConversation { .. }
            | AgentChatIntentFrame::SendToConversation { .. }
            | AgentChatIntentFrame::WaitForConversations { .. }
            | AgentChatIntentFrame::ListLinkedConversations { .. }
    )
}
