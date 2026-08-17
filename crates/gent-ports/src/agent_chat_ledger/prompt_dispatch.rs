//! Durable claim and settlement ownership for provider-bound agent-chat prompts.

use gent_types::{AgentChatPromptSaved, AgentChatProvider, HostEpoch};

use crate::LedgerError;

/// Reads and transitions the durable outbox populated only for `SendPrompt` requests.
pub trait AgentChatPromptDispatchLedger: Send + Sync {
    /// Atomically claims the oldest pre-launch prompt for this daemon epoch.
    /// # Errors
    /// Returns an error when ingress is closed, the epoch is stale, or persistence fails.
    fn claim_agent_chat_prompt_dispatch(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError>;

    /// Records the durable point after which a provider invocation may be ambiguous.
    ///
    /// Once this succeeds, crash recovery must never replay the prompt automatically.
    /// # Errors
    /// Returns an error when current durable ownership does not hold.
    fn begin_agent_chat_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    /// Confirms that the provider process was launched under the current durable owner.
    /// # Errors
    /// Returns an error when the launch marker or owner fence no longer matches.
    fn confirm_agent_chat_prompt_started(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    /// Returns a prompt to the outbox only while it is still known not to have launched.
    ///
    /// # Errors
    /// Returns an error when the host fence, owner binding, or durable transition rejects it.
    fn release_agent_chat_prompt_claim(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    /// Returns a launch-marked prompt only after a local result proves no runner was invoked.
    /// # Errors
    /// Returns an error when the launch marker or owner fence no longer matches.
    fn release_agent_chat_prompt_unstarted_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    /// Retires an ambiguous launch without permitting automatic replay.
    /// # Errors
    /// Returns an error when the launch marker or owner fence no longer matches.
    fn mark_agent_chat_prompt_unprovable(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    /// Permanently settles one started provider prompt only for its current owner.
    ///
    /// # Errors
    /// Returns an error when the host fence, owner binding, or durable transition rejects it.
    fn settle_agent_chat_prompt_dispatch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    /// Recovers pre-launch work after an already-fenced successor opens a new host epoch.
    ///
    /// Old `claimed` rows become pending; old `launching` or `started` rows become unprovable.
    /// # Errors
    /// Returns an error when the supplied epoch is not the open successor epoch.
    fn recover_agent_chat_prompt_dispatches(
        &self,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;
}
