use gent_types::{
    AgentChatPromptSaved, AgentChatProvider, AgentChatRunId, Command, DurableTurnPhase, Event,
    HostEpoch, PromptHoldReason, ProviderPromptReadinessBinding,
    ProviderPromptReadinessFailureBinding, Receipt, ReceiptId,
};

use crate::LedgerError;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PromptAdmission {
    Held,
    Installing,
    Admitted,
}

pub trait AgentChatPromptDispatchLedger: Send + Sync {
    fn read_awaiting_agent_chat_prompts(
        &self,
        _: Option<&str>,
        _: &str,
        _: u16,
    ) -> Result<Vec<AgentChatPromptSaved>, LedgerError> {
        Err(LedgerError::Invariant(
            "durable held prompt inspection is unavailable".into(),
        ))
    }

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

    fn hold_agent_chat_prompt_for_admission(
        &self,
        _: &ReceiptId,
        _: HostEpoch,
        _: PromptHoldReason,
    ) -> Result<(), LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat prompt admission hold is unavailable".into(),
        ))
    }

    fn agent_chat_prompt_admission(&self, _: &ReceiptId) -> Result<PromptAdmission, LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat prompt admission state is unavailable".into(),
        ))
    }

    fn agent_chat_prompt_turn_id(&self, _: &ReceiptId) -> Result<String, LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat prompt turn identity is unavailable".into(),
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

    /// # Errors
    /// Returns an error when ingress is closed, the epoch is stale, or persistence fails.
    fn claim_agent_chat_prompt_dispatch(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError>;

    fn claim_agent_chat_prompt_dispatch_excluding_runs(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
        excluded_run_ids: &[AgentChatRunId],
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
        if excluded_run_ids.is_empty() {
            return self.claim_agent_chat_prompt_dispatch(coordinator_id, host_epoch, provider);
        }
        Err(LedgerError::Invariant(
            "agent chat excluded-run prompt claim is unavailable".into(),
        ))
    }

    fn has_pending_agent_chat_prompt_dispatch(
        &self,
        _: AgentChatProvider,
    ) -> Result<bool, LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat pending-dispatch inspection is unavailable".into(),
        ))
    }

    /// # Errors
    /// Returns an error when current durable ownership does not hold.
    fn begin_agent_chat_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    /// # Errors
    /// Returns an error when the launch marker or owner fence no longer matches.
    fn confirm_agent_chat_prompt_started(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

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
        _: Option<gent_types::AgentChatFailureCode>,
    ) -> Result<(), LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat prelaunch failure settlement is unavailable".into(),
        ))
    }

    fn record_agent_chat_prompt_failure_notice(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
        _: &str,
        _: Option<gent_types::AgentChatFailureCode>,
    ) -> Result<(), LedgerError> {
        Err(LedgerError::Invariant(
            "owned launch failure notices are unavailable".into(),
        ))
    }

    /// # Errors
    /// Returns an error when the launch marker or owner fence no longer matches.
    fn release_agent_chat_prompt_unstarted_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

    fn agent_chat_prompt_has_steer_target(&self, _: &str) -> Result<bool, LedgerError> {
        Err(LedgerError::Invariant(
            "steer target lookup is unavailable".into(),
        ))
    }

    fn claim_steered_agent_chat_prompt_for_turn(
        &self,
        _: &str,
        _: HostEpoch,
        _: &AgentChatRunId,
        _: &str,
    ) -> Result<Option<(AgentChatPromptSaved, ReceiptId)>, LedgerError> {
        Err(LedgerError::Invariant(
            "targeted steer claim is unavailable".into(),
        ))
    }

    fn claim_steered_agent_chat_prompt(
        &self,
        _: &str,
        _: HostEpoch,
        _: &AgentChatRunId,
    ) -> Result<Option<(AgentChatPromptSaved, ReceiptId)>, LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat steered prompt claim is unavailable".into(),
        ))
    }

    fn deliver_steered_agent_chat_prompt(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
        _: &str,
    ) -> Result<(), LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat steered prompt delivery is unavailable".into(),
        ))
    }

    fn start_steered_agent_chat_prompt_turn(
        &self,
        _: &str,
        _: &str,
        _: HostEpoch,
    ) -> Result<(), LedgerError> {
        Err(LedgerError::Invariant(
            "agent chat steered prompt turn start is unavailable".into(),
        ))
    }

    /// # Errors
    /// Returns an error when the launch marker or owner fence no longer matches.
    fn mark_agent_chat_prompt_unprovable(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;

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

    fn interrupt_unstarted_agent_chat_prompt(
        &self,
        _: &str,
        _: Option<&str>,
        _: HostEpoch,
    ) -> Result<bool, LedgerError> {
        Err(LedgerError::Invariant(
            "unstarted agent chat prompt interruption is unavailable".into(),
        ))
    }

    /// # Errors
    /// Returns an error when the supplied epoch is not the open successor epoch.
    fn recover_agent_chat_prompt_dispatches(
        &self,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError>;
}
