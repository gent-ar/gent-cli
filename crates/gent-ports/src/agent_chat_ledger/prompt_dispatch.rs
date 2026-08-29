//! Durable claim and settlement ownership for provider-bound agent-chat prompts.

use gent_types::{
    AgentChatPromptSaved, AgentChatProvider, AgentChatRunId, Command, DurableTurnPhase, Event,
    HostEpoch, ProviderPromptReadinessBinding, ProviderPromptReadinessFailureBinding, Receipt,
};

use crate::LedgerError;

/// Reads and transitions the durable outbox populated only for `SendPrompt` requests.
pub trait AgentChatPromptDispatchLedger: Send + Sync {
    /// Atomically records one verified provider-ready decision and releases its exact held prompt.
    ///
    /// The command binds the prompt receipt, conversation, current run, and selected provider. A
    /// matching retry returns the original settled receipt without changing dispatch ownership.
    /// This transition only makes the prompt claimable; it never wakes or starts a provider.
    ///
    /// # Errors
    /// Returns when the proof, current-run/provider fence, epoch, or terminal event differs.
    fn release_verified_agent_chat_prompt_after_readiness(
        &self,
        _: &Command,
        _: &Event,
        _: &ProviderPromptReadinessBinding,
    ) -> Result<Receipt, LedgerError> {
        Err(LedgerError::Invariant(
            "verified agent chat prompt readiness release is unavailable".into(),
        ))
    }

    fn fail_verified_agent_chat_prompt_after_readiness(
        &self,
        _: &Command,
        _: &Event,
        _: &ProviderPromptReadinessFailureBinding,
    ) -> Result<Receipt, LedgerError> {
        Err(LedgerError::Invariant(
            "verified agent chat prompt readiness failure is unavailable".into(),
        ))
    }

    /// Releases one exact prompt only after daemon-owned provider readiness is durably proven.
    ///
    /// The default fails closed so generic ledger fakes cannot bypass the readiness boundary.
    /// # Errors
    /// Returns when the implementation cannot atomically retain the current run fence.
    fn release_agent_chat_prompt_after_readiness(
        &self,
        _: &str,
        _: &AgentChatRunId,
        _: HostEpoch,
    ) -> Result<(), LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat prompt readiness release is unavailable".into(),
        ))
    }

    /// Atomically claims the oldest pre-launch prompt for this daemon epoch.
    /// # Errors
    /// Returns an error when ingress is closed, the epoch is stale, or persistence fails.
    fn claim_agent_chat_prompt_dispatch(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError>;

    /// Reports whether the current selection for `provider` has durable
    /// pre-launch work waiting in the outbox. This does not claim work or
    /// start a provider.
    fn has_pending_agent_chat_prompt_dispatch(
        &self,
        _: AgentChatProvider,
    ) -> Result<bool, LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat pending-dispatch inspection is unavailable".into(),
        ))
    }

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

    fn fail_agent_chat_prompt_prelaunch(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
        _: &str,
    ) -> Result<(), LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat prelaunch failure settlement is unavailable".into(),
        ))
    }

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

    fn settle_agent_chat_prompt_terminal(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
        _: DurableTurnPhase,
    ) -> Result<(), LedgerError> {
        Err(LedgerError::Invariant(
            "atomic agent chat terminal settlement is unavailable".into(),
        ))
    }

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
