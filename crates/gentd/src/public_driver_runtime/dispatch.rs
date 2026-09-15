use gent_ports::{
    AgentChatPromptDispatchLedger, ConversationActivityLedger, Ledger, PublicProviderResolver,
    PublicProviderRunner, TranscriptLedger,
};
use gent_runtime::{AgentChatPromptDispatchResult, RuntimeError};
use gent_types::{AgentChatProvider, AgentChatRunId, HostEpoch};

use super::PublicDriversRuntime;

impl<L, D, R> PublicDriversRuntime<L, D, R>
where
    L: Clone
        + Ledger
        + gent_ports::RunLifecycleFactLedger
        + ConversationActivityLedger
        + TranscriptLedger
        + AgentChatPromptDispatchLedger,
    D: PublicProviderRunner + Clone,
    R: PublicProviderResolver,
{
    /// Claims the next durable prompt only for this approved daemon lifecycle owner.
    ///
    /// # Errors
    /// Returns an error when the durable ownership fence rejects the claim.
    pub(crate) fn claim_prompt(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<AgentChatPromptDispatchResult, RuntimeError> {
        self.dispatches.claim(coordinator_id, host_epoch, provider)
    }

    pub(crate) fn claim_prompt_excluding_runs(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
        excluded_run_ids: &[AgentChatRunId],
    ) -> Result<AgentChatPromptDispatchResult, RuntimeError> {
        self.dispatches
            .claim_excluding_runs(coordinator_id, host_epoch, provider, excluded_run_ids)
    }

    /// Marks the durable boundary before a provider process may be launched.
    ///
    /// # Errors
    /// Returns an error when durable ownership validation rejects the release.
    pub(crate) fn begin_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .begin_launch(message_id, coordinator_id, host_epoch)
    }

    /// Confirms that the daemon-owned runner successfully launched the provider process.
    ///
    /// # Errors
    /// Returns an error when durable ownership validation rejects settlement.
    pub(crate) fn confirm_prompt_started(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .confirm_started(message_id, coordinator_id, host_epoch)
    }

    /// Returns a claim to the durable outbox before a launch boundary is crossed.
    pub(crate) fn release_prompt_claim(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .release_claim(message_id, coordinator_id, host_epoch)
    }

    /// Returns a launch marker only after a local result proves no provider runner was called.
    pub(crate) fn release_unstarted_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .release_unstarted_launch(message_id, coordinator_id, host_epoch)
    }

    /// Retires an ambiguous launch without allowing the durable prompt to replay automatically.
    pub(crate) fn mark_prompt_unprovable(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .mark_unprovable(message_id, coordinator_id, host_epoch)
    }

    pub(crate) fn settle_prompt_terminal(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        phase: gent_types::DurableTurnPhase,
    ) -> Result<(), RuntimeError> {
        self.dispatches
            .settle_terminal(message_id, coordinator_id, host_epoch, phase)
    }

    /// Recovers only pre-launch work after a successor daemon has fenced the previous epoch.
    pub(crate) fn recover_prompts(&self, host_epoch: HostEpoch) -> Result<(), RuntimeError> {
        self.dispatches.recover(host_epoch)
    }
}
