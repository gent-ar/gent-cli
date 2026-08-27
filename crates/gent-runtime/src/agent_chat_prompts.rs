//! Authority-gated persistence of one agent-chat user prompt.
//!
//! This wrapper deliberately has no provider, process, or daemon dependency. The prompt ledger
//! owns receipt correlation, current-run resolution, and the single durable transaction.

use gent_ports::AgentChatPromptLedger;
use gent_types::{
    AgentChatConversationId, AgentChatPromptCreate, AgentChatPromptDisposition,
    AgentChatPromptSaved, AgentChatRequestId, AgentChatRunId, HostEpoch, ReceiptId,
};

use crate::RuntimeError;

/// Explicit permission to create local agent-chat prompt state.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatPromptAuthority {
    /// Observer behavior performs no receipt claim, prompt write, or transcript change.
    #[default]
    Observer,
    /// Reserved for the future approved single writer.
    Approved,
}

/// Protocol-neutral input for one user prompt; no provider-native value crosses this boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentChatPromptRequest {
    pub request_id: AgentChatRequestId,
    pub receipt_id: ReceiptId,
    pub host_epoch: HostEpoch,
    pub conversation_id: AgentChatConversationId,
    pub disposition: AgentChatPromptDisposition,
    pub text: String,
    pub attachment_ids: Vec<String>,
    pub tool_source_ids: Vec<String>,
}

/// A no-write observer denial or the settled prompt receipt, message, and resolved run.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatPromptResult {
    DeniedObserver,
    Saved(Box<AgentChatPromptSaved>),
}

/// Maps a prompt request to the dedicated atomic prompt ledger only after authority approval.
#[derive(Clone, Debug)]
pub struct AgentChatPromptService<L> {
    ledger: L,
    authority: AgentChatPromptAuthority,
}

impl<L> AgentChatPromptService<L> {
    /// Builds an inert observer service unless the future writer is explicitly approved.
    #[must_use]
    pub fn new(ledger: L, authority: AgentChatPromptAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: AgentChatPromptLedger> AgentChatPromptService<L> {
    /// Persists one user prompt without starting, signaling, or resolving a provider.
    ///
    /// # Errors
    /// Returns an error only after approved authority reaches the durable prompt ledger.
    pub fn submit(
        &self,
        request: &AgentChatPromptRequest,
    ) -> Result<AgentChatPromptResult, RuntimeError> {
        gent_types::validate_tool_source_ids(&request.tool_source_ids)
            .map_err(|error| gent_ports::LedgerError::Invariant(error.to_string()))?;
        if self.authority != AgentChatPromptAuthority::Approved {
            return Ok(AgentChatPromptResult::DeniedObserver);
        }
        Ok(AgentChatPromptResult::Saved(Box::new(
            self.ledger.save_agent_chat_prompt(&to_create(request))?,
        )))
    }

    /// Persists a prompt only when the exact reviewed run remains current.
    ///
    /// This is a private composition seam for provider-readiness/consent. Ordinary prompt
    /// persistence deliberately keeps its existing current-run behavior until that authority is
    /// composed and advertised.
    ///
    /// # Errors
    /// Returns when durable prompt ownership or the exact current-run fence cannot be verified.
    pub fn submit_for_run(
        &self,
        request: &AgentChatPromptRequest,
        expected_run_id: &AgentChatRunId,
    ) -> Result<AgentChatPromptResult, RuntimeError> {
        gent_types::validate_tool_source_ids(&request.tool_source_ids)
            .map_err(|error| gent_ports::LedgerError::Invariant(error.to_string()))?;
        if self.authority != AgentChatPromptAuthority::Approved {
            return Ok(AgentChatPromptResult::DeniedObserver);
        }
        Ok(AgentChatPromptResult::Saved(Box::new(
            self.ledger
                .save_agent_chat_prompt_for_run(&to_create(request), expected_run_id)?,
        )))
    }
}

fn to_create(request: &AgentChatPromptRequest) -> AgentChatPromptCreate {
    AgentChatPromptCreate {
        request_id: request.request_id.clone(),
        receipt_id: request.receipt_id.clone(),
        host_epoch: request.host_epoch,
        conversation_id: request.conversation_id.clone(),
        disposition: request.disposition,
        text: request.text.clone(),
        attachment_ids: request.attachment_ids.clone(),
        tool_source_ids: request.tool_source_ids.clone(),
    }
}
