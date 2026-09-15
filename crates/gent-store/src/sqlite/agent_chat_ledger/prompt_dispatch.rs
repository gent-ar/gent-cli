//! `SQLite` outbox ownership for durable provider-bound agent-chat prompts.

use gent_ports::{AgentChatPromptDispatchLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatPromptSaved, AgentChatProvider, AgentChatRunId, Command, DurableTurnPhase, Event,
    HostEpoch, ProviderPromptReadinessBinding, ProviderPromptReadinessFailureBinding, Receipt,
};
use rusqlite::Transaction;

use super::super::SqliteLedger;
use super::super::epoch::require_epoch;
use super::super::queries::host_ingress;
use super::prompt_dispatch_readiness;

impl AgentChatPromptDispatchLedger for SqliteLedger {
    fn claim_agent_chat_prompt_dispatch(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
        claim_excluding_runs(self, coordinator_id, host_epoch, provider, &[])
    }

    fn claim_agent_chat_prompt_dispatch_excluding_runs(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
        excluded_run_ids: &[AgentChatRunId],
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
        claim_excluding_runs(self, coordinator_id, host_epoch, provider, excluded_run_ids)
    }

    fn has_pending_agent_chat_prompt_dispatch(
        &self,
        provider: AgentChatProvider,
    ) -> Result<bool, LedgerError> {
        helpers::has_pending(self, provider)
    }

    fn release_agent_chat_prompt_after_readiness(
        &self,
        message_id: &str,
        expected_run_id: &AgentChatRunId,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        prompt_dispatch_readiness::release(self, message_id, expected_run_id, host_epoch)
    }

    fn hold_agent_chat_prompt_for_admission(
        &self,
        prompt_receipt_id: &gent_types::ReceiptId,
        host_epoch: HostEpoch,
        reason: gent_types::PromptHoldReason,
    ) -> Result<(), LedgerError> {
        super::prompt_admission_hold::hold(self, prompt_receipt_id, host_epoch, reason)
    }

    fn agent_chat_prompt_admission(
        &self,
        prompt_receipt_id: &gent_types::ReceiptId,
    ) -> Result<gent_ports::PromptAdmission, LedgerError> {
        super::prompt_admission_hold::admission(self, prompt_receipt_id)
    }

    fn release_verified_agent_chat_prompt_after_readiness(
        &self,
        command: &Command,
        terminal: &Event,
        binding: &ProviderPromptReadinessBinding,
    ) -> Result<Receipt, LedgerError> {
        prompt_dispatch_readiness::release_verified(self, command, terminal, binding)
    }

    fn fail_verified_agent_chat_prompt_after_readiness(
        &self,
        command: &Command,
        terminal: &Event,
        binding: &ProviderPromptReadinessFailureBinding,
    ) -> Result<Receipt, LedgerError> {
        prompt_dispatch_readiness::fail_verified(self, command, terminal, binding)
    }

    fn begin_agent_chat_prompt_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        helpers::transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "claimed",
            "launching",
            true,
        )
    }

    fn confirm_agent_chat_prompt_started(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        helpers::transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "launching",
            "started",
            true,
        )
    }

    fn release_agent_chat_prompt_claim(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        helpers::transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "claimed",
            "pending",
            false,
        )
    }

    fn fail_agent_chat_prompt_prelaunch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        error: &str,
    ) -> Result<(), LedgerError> {
        helpers::fail_prelaunch(self, message_id, coordinator_id, host_epoch, error)
    }

    fn release_agent_chat_prompt_unstarted_launch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        helpers::transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "launching",
            "pending",
            false,
        )
    }

    fn claim_steered_agent_chat_prompt(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        run_id: &AgentChatRunId,
    ) -> Result<Option<(AgentChatPromptSaved, gent_types::ReceiptId)>, LedgerError> {
        steer::claim(self, coordinator_id, host_epoch, run_id)
    }

    fn deliver_steered_agent_chat_prompt(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        turn_id: &str,
    ) -> Result<(), LedgerError> {
        steer::deliver(self, message_id, coordinator_id, host_epoch, turn_id)
    }

    fn start_steered_agent_chat_prompt_turn(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        steer::start_turn(self, message_id, coordinator_id, host_epoch)
    }

    fn mark_agent_chat_prompt_unprovable(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        terminal::abandon_unprovable(self, message_id, coordinator_id, host_epoch)
    }

    fn settle_agent_chat_prompt_dispatch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        helpers::transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "started",
            "settled",
            true,
        )
    }

    fn settle_agent_chat_prompt_terminal(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        phase: DurableTurnPhase,
    ) -> Result<(), LedgerError> {
        terminal::settle(self, message_id, coordinator_id, host_epoch, phase)
    }

    fn recover_agent_chat_prompt_dispatches(
        &self,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        prompt_dispatch_recovery::recover(self, host_epoch)
    }
}

#[path = "prompt_dispatch_helpers.rs"]
mod helpers;

#[path = "prompt_dispatch_claim.rs"]
mod claim;
use claim::claim_excluding_runs;

#[path = "prompt_dispatch_terminal.rs"]
mod terminal;

#[path = "prompt_steer.rs"]
mod steer;

#[path = "prompt_dispatch_recovery.rs"]
mod prompt_dispatch_recovery;

pub(super) fn require_open(
    transaction: &Transaction<'_>,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let ingress = host_ingress(transaction)?;
    require_epoch(host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    Ok(())
}
