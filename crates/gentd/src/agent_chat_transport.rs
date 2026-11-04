//! Capability-gated agent-chat intent transport.
//!
//! This adapter owns JSON framing and response correlation only. It does not create a
//! conversation, persist a prompt, or launch a provider; those authority decisions stay behind
//! [`RuntimeApi`].

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AGENT_CHAT_PROVIDER_WARM_CAPABILITY,
    AGENT_CHAT_TRANSCRIPT_IMPORT_CAPABILITY, AgentChatIntentFrame,
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
#[cfg(test)]
#[path = "targeted_steer_transport_tests.rs"]
mod targeted_steer_tests;

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
    let frame = serde_json::from_value::<AgentChatIntentFrame>(raw.clone()).ok()?;
    let off_thread = match frame {
        AgentChatIntentFrame::WaitForConversations { .. } => {
            supports(capabilities) && links_enabled(capabilities)
        }
        AgentChatIntentFrame::WarmProvider(_) => warm_enabled(capabilities),
        _ => false,
    };
    off_thread.then_some(frame)
}

fn warm_enabled(capabilities: &CapabilitySet) -> bool {
    capabilities
        .0
        .iter()
        .any(|capability| capability == AGENT_CHAT_PROVIDER_WARM_CAPABILITY)
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
    if matches!(request, AgentChatIntentFrame::SteerPromptForTurn(_))
        && !capabilities
            .0
            .iter()
            .any(|v| v == gent_protocol::AGENT_CHAT_STEER_ENQUEUE_CAPABILITY)
    {
        write_error(
            stream,
            "unsupportedCapability",
            "atomic steer enqueue was not negotiated",
        )
        .await?;
        return Ok(true);
    }
    if matches!(
        request,
        AgentChatIntentFrame::SteerQueuedPromptForTurn { .. }
            | AgentChatIntentFrame::SteerPromptForTurn(_)
    ) && !capabilities
        .0
        .iter()
        .any(|v| v == gent_protocol::AGENT_CHAT_STEER_TURN_CAPABILITY)
    {
        write_error(
            stream,
            "unsupportedCapability",
            "turn-bound steering was not negotiated",
        )
        .await?;
        return Ok(true);
    }
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
    if matches!(request, AgentChatIntentFrame::WarmProvider(_)) && !warm_enabled(capabilities) {
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
            | AgentChatIntentFrame::SteerQueuedPromptForTurn { .. }
            | AgentChatIntentFrame::SteerPromptForTurn(_)
            | AgentChatIntentFrame::ContinueFromSavedHistory { .. }
            | AgentChatIntentFrame::SwitchSelection { .. }
            | AgentChatIntentFrame::Interrupt { .. }
            | AgentChatIntentFrame::WarmProvider(_)
            | AgentChatIntentFrame::Decision { .. }
            | AgentChatIntentFrame::Subscribe { .. }
            | AgentChatIntentFrame::ForkConversation { .. }
            | AgentChatIntentFrame::CreateLinkedConversation { .. }
            | AgentChatIntentFrame::SendToConversation { .. }
            | AgentChatIntentFrame::WaitForConversations { .. }
            | AgentChatIntentFrame::ListLinkedConversations { .. }
    )
}
