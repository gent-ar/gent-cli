//! Durable claim and settlement ownership for provider-bound agent-chat prompts.

use gent_types::{AgentChatPromptSaved, AgentChatProvider, HostEpoch};

use crate::LedgerError;

/// Reads and transitions the durable outbox populated only for `SendPrompt` requests.
pub trait AgentChatPromptDispatchLedger: Send + Sync {
    /// Atomically claims the oldest runnable prompt for this daemon epoch.
    ///
    /// A successor may reclaim a claim from an earlier epoch after it has fenced the old host.
    /// # Errors
    /// Returns an error when ingress is closed, the epoch is stale, or persistence fails.
    fn claim_agent_chat_prompt_dispatch(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError>;

    /// Returns a failed-to-start prompt to the durable outbox only for its current owner.
    ///
    /// # Errors
    /// Returns an error when the host fence, owner binding, or durable transition rejects it.
    fn release_agent_chat_prompt_dispatch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    /// Permanently settles one provider-owned prompt only for its current owner.
    ///
    /// # Errors
    /// Returns an error when the host fence, owner binding, or durable transition rejects it.
    fn settle_agent_chat_prompt_dispatch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;
}
