//! Capability-gated agent-chat intent transport.
//!
//! This adapter owns JSON framing and response correlation only. It does not create a
//! conversation, persist a prompt, or launch a provider; those authority decisions stay behind
//! [`RuntimeApi`].

use gent_protocol::{
    AGENT_CHAT_INTENTS_CAPABILITY, AgentChatIntentFrame, AgentChatSubscriptionEnd, write_json_frame,
};
use gent_types::CapabilitySet;
use serde_json::Value;
use tokio::io::AsyncWrite;

use crate::{api::RuntimeApi, transport::write_error};

pub(crate) trait IntentPort {
    fn exchange(&self, request: AgentChatIntentFrame) -> Result<Vec<AgentChatIntentFrame>, String>;
}

impl<R: RuntimeApi> IntentPort for R {
    fn exchange(&self, request: AgentChatIntentFrame) -> Result<Vec<AgentChatIntentFrame>, String> {
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
    dispatch_port(stream, runtime, capabilities, raw).await
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
        Err(message) => write_error(stream, "agentChatRejected", &message).await?,
    }
    Ok(true)
}

fn supports(capabilities: &CapabilitySet) -> bool {
    capabilities
        .0
        .iter()
        .any(|capability| capability == AGENT_CHAT_INTENTS_CAPABILITY)
}

fn is_client_request(frame: &AgentChatIntentFrame) -> bool {
    matches!(
        frame,
        AgentChatIntentFrame::CreateConversation { .. }
            | AgentChatIntentFrame::SendPrompt { .. }
            | AgentChatIntentFrame::QueuePrompt { .. }
            | AgentChatIntentFrame::Interrupt { .. }
            | AgentChatIntentFrame::Decision { .. }
            | AgentChatIntentFrame::Subscribe { .. }
    )
}

fn validate_replies(
    request: &AgentChatIntentFrame,
    replies: &[AgentChatIntentFrame],
) -> Result<(), &'static str> {
    match request {
        AgentChatIntentFrame::Subscribe {
            request_id,
            after_cursor,
            ..
        } => validate_subscription(request_id, *after_cursor, replies),
        AgentChatIntentFrame::CreateConversation {
            request_id,
            receipt_id,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::Created { request_id: reply_id, receipt, conversation_id, run_id }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
                    && !conversation_id.0.is_empty() && !run_id.0.is_empty()
        )
        .then_some(())
        .ok_or("conversation creation requires one matching durable result"),
        AgentChatIntentFrame::SendPrompt {
            request_id,
            receipt_id,
            ..
        }
        | AgentChatIntentFrame::QueuePrompt {
            request_id,
            receipt_id,
            ..
        }
        | AgentChatIntentFrame::Interrupt {
            request_id,
            receipt_id,
            ..
        }
        | AgentChatIntentFrame::Decision {
            request_id,
            receipt_id,
            ..
        } => matches!(
            replies,
            [AgentChatIntentFrame::Accepted { request_id: reply_id, receipt, .. }]
                if reply_id == request_id && receipt.receipt_id == *receipt_id
        )
        .then_some(())
        .ok_or("a chat command requires one matching accepted receipt"),
        AgentChatIntentFrame::SubscriptionEvent { .. }
        | AgentChatIntentFrame::SubscriptionEnded { .. }
        | AgentChatIntentFrame::Created { .. }
        | AgentChatIntentFrame::Accepted { .. } => {
            Err("agent chat response frames are server-only")
        }
    }
}

fn validate_subscription(
    request_id: &gent_types::AgentChatRequestId,
    after_cursor: u64,
    replies: &[AgentChatIntentFrame],
) -> Result<(), &'static str> {
    let Some((last, events)) = replies.split_last() else {
        return Err("a chat subscription must end explicitly");
    };
    if !matches!(last, AgentChatIntentFrame::SubscriptionEnded { request_id: reply_id, reason: AgentChatSubscriptionEnd::ServerClosing | AgentChatSubscriptionEnd::ResyncRequired } if reply_id == request_id)
    {
        return Err("a chat subscription must end with its matching terminal frame");
    }
    let mut cursor = after_cursor;
    for event in events {
        let AgentChatIntentFrame::SubscriptionEvent {
            request_id: reply_id,
            event,
        } = event
        else {
            return Err("chat subscription replies may contain only events before termination");
        };
        if reply_id != request_id || event.cursor <= cursor {
            return Err("chat subscription event correlation or cursor is invalid");
        }
        cursor = event.cursor;
    }
    Ok(())
}
