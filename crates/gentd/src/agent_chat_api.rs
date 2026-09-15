//! Maps supported agent-chat intent frames onto authority-gated runtime services.

use gent_protocol::AgentChatIntentFrame;

use crate::agent_chat_intent_error::AgentChatIntentError;
use gent_runtime::{
    AgentChatConversationService, AgentChatForkService, AgentChatPromptService,
    AgentChatSelectionGate, AgentChatSelectionSwitchService,
};
use gent_types::{
    AgentChatConversationId, AgentChatPromptDisposition, AgentChatRunId, HostEpoch, ReceiptId,
};
#[path = "agent_chat_api_create.rs"]
mod create;
use create::create;
#[path = "agent_chat_api_fork.rs"]
mod fork;
use fork::fork;
#[path = "agent_chat_api_prompt.rs"]
mod prompt;
use prompt::prompt;
#[path = "agent_chat_api_switch.rs"]
mod switch;
use switch::{SwitchInput, switch};

/// Daemon-composition notification issued only after a prompt transaction commits.
///
/// Implementations may arm a bounded private lifecycle owner, but must never start a provider
/// inline or report a provider-native session to this durable chat adapter.
pub(crate) trait PromptCommitWake {
    type Error;

    /// Whether this explicit composition may assess a newly held prompt before waking a host.
    ///
    /// The default keeps generic and observer-facing chat persistence inert. Only a daemon-owned
    /// readiness admission may opt in, and it must retain the prompt when readiness is not proven.
    fn handles_awaiting_readiness(&self) -> bool {
        false
    }

    fn wake_after_prompt_commit(&mut self, prompt: PromptWake) -> Result<(), Self::Error>;
}

/// Durable-only identity delivered to an authority-owned lifecycle router after a prompt commit.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct PromptWake {
    pub(crate) conversation_id: AgentChatConversationId,
    pub(crate) run_id: AgentChatRunId,
    pub(crate) receipt_id: ReceiptId,
    pub(crate) disposition: AgentChatPromptDisposition,
}

struct NoopPromptCommitWake;

impl PromptCommitWake for NoopPromptCommitWake {
    type Error = std::convert::Infallible;

    fn wake_after_prompt_commit(&mut self, _: PromptWake) -> Result<(), Self::Error> {
        Ok(())
    }
}

/// Handles the durable subset available before provider lifecycle composition.
pub(crate) fn exchange<L, C, S>(
    conversations: &AgentChatConversationService<L, C>,
    prompts: &AgentChatPromptService<L>,
    switches: &AgentChatSelectionSwitchService<L, S>,
    forks: &AgentChatForkService<L>,
    host_epoch: HostEpoch,
    frame: AgentChatIntentFrame,
) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError>
where
    L: gent_ports::AgentChatLedger
        + gent_ports::AgentChatWorkspaceLedger
        + gent_ports::AgentChatPromptLedger
        + gent_ports::AgentChatSelectionLedger
        + gent_ports::AgentChatForkLedger,
    C: AgentChatSelectionGate,
    S: AgentChatSelectionGate,
{
    exchange_with_wake(
        conversations,
        prompts,
        switches,
        forks,
        host_epoch,
        frame,
        &mut NoopPromptCommitWake,
    )
}

/// Handles one finite exchange without making a retained prompt lifecycle-claimable.
///
/// This remains a composition seam rather than a transport capability. A future private
/// readiness authority alone may release and wake a held prompt after it proves the provider.
pub(crate) fn exchange_with_wake<L, C, S, W>(
    conversations: &AgentChatConversationService<L, C>,
    prompts: &AgentChatPromptService<L>,
    switches: &AgentChatSelectionSwitchService<L, S>,
    forks: &AgentChatForkService<L>,
    host_epoch: HostEpoch,
    frame: AgentChatIntentFrame,
    wake: &mut W,
) -> Result<Vec<AgentChatIntentFrame>, AgentChatIntentError>
where
    L: gent_ports::AgentChatLedger
        + gent_ports::AgentChatWorkspaceLedger
        + gent_ports::AgentChatForkLedger
        + gent_ports::AgentChatPromptLedger
        + gent_ports::AgentChatSelectionLedger,
    C: AgentChatSelectionGate,
    S: AgentChatSelectionGate,
    W: PromptCommitWake,
{
    match frame {
        AgentChatIntentFrame::CreateConversation {
            request_id,
            receipt_id,
            workspace_path,
            selection,
        } => create(
            conversations,
            host_epoch,
            request_id,
            receipt_id,
            &workspace_path,
            selection.ok_or("agent-chat conversation creation requires a model selection")?,
        ),
        AgentChatIntentFrame::SendPrompt {
            request_id,
            receipt_id,
            conversation_id,
            text,
            attachment_ids,
        } => prompt(
            prompts,
            host_epoch,
            PromptInput {
                request_id,
                receipt_id,
                conversation_id,
                text,
                attachment_ids,
                tool_source_ids: Vec::new(),
                disposition: AgentChatPromptDisposition::Send,
            },
            wake,
        ),
        AgentChatIntentFrame::QueuePrompt {
            request_id,
            receipt_id,
            conversation_id,
            text,
            attachment_ids,
        } => prompt(
            prompts,
            host_epoch,
            PromptInput {
                request_id,
                receipt_id,
                conversation_id,
                text,
                attachment_ids,
                tool_source_ids: Vec::new(),
                disposition: AgentChatPromptDisposition::Queue,
            },
            wake,
        ),
        AgentChatIntentFrame::SendPromptWithTools {
            request_id,
            receipt_id,
            conversation_id,
            text,
            attachment_ids,
            tool_source_ids,
        } => prompt(
            prompts,
            host_epoch,
            PromptInput {
                request_id,
                receipt_id,
                conversation_id,
                text,
                attachment_ids,
                tool_source_ids,
                disposition: AgentChatPromptDisposition::Send,
            },
            wake,
        ),
        AgentChatIntentFrame::QueuePromptWithTools {
            request_id,
            receipt_id,
            conversation_id,
            text,
            attachment_ids,
            tool_source_ids,
        } => prompt(
            prompts,
            host_epoch,
            PromptInput {
                request_id,
                receipt_id,
                conversation_id,
                text,
                attachment_ids,
                tool_source_ids,
                disposition: AgentChatPromptDisposition::Queue,
            },
            wake,
        ),
        AgentChatIntentFrame::SwitchSelection {
            request_id,
            receipt_id,
            conversation_id,
            parent_run_id,
            selection,
            context_policy,
        } => switch(
            switches,
            host_epoch,
            SwitchInput {
                request_id,
                receipt_id,
                conversation_id,
                parent_run_id,
                selection,
                context_policy,
            },
        ),
        AgentChatIntentFrame::ForkConversation {
            request_id,
            receipt_id,
            source_conversation_id,
            fork_through_message_id,
        } => fork(
            forks,
            host_epoch,
            request_id,
            receipt_id,
            source_conversation_id,
            fork_through_message_id,
        ),
        AgentChatIntentFrame::Interrupt { .. } | AgentChatIntentFrame::Decision { .. } => {
            Err("agent-chat provider lifecycle is not configured".into())
        }
        AgentChatIntentFrame::Subscribe { .. } => {
            Err("agent-chat transcript streaming is not configured".into())
        }
        _ => Err("agent-chat response frames are server-only".into()),
    }
}

struct PromptInput {
    request_id: gent_types::AgentChatRequestId,
    receipt_id: gent_types::ReceiptId,
    conversation_id: gent_types::AgentChatConversationId,
    text: String,
    attachment_ids: Vec<String>,
    disposition: AgentChatPromptDisposition,
    tool_source_ids: Vec<String>,
}

#[cfg(test)]
#[path = "agent_chat_api_tests.rs"]
mod tests;
