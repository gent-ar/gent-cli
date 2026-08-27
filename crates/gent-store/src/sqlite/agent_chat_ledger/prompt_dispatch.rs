//! `SQLite` outbox ownership for durable provider-bound agent-chat prompts.

use gent_ports::{AgentChatPromptDispatchLedger, IngressMode, LedgerError};
use gent_types::{
    AgentChatPromptSaved, AgentChatProvider, AgentChatRunId, Command, DurableTurnPhase, Event,
    HostEpoch, ProviderPromptReadinessBinding, ProviderPromptReadinessFailureBinding, Receipt,
};
use rusqlite::{OptionalExtension, Transaction, TransactionBehavior, params};

use super::super::SqliteLedger;
use super::super::epoch::require_epoch;
use super::super::queries::{host_ingress, storage_error};
use super::prompt_dispatch_readiness;

impl AgentChatPromptDispatchLedger for SqliteLedger {
    fn claim_agent_chat_prompt_dispatch(
        &self,
        coordinator_id: &str,
        host_epoch: HostEpoch,
        provider: AgentChatProvider,
    ) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
        claim(self, coordinator_id, host_epoch, provider)
    }

    fn release_agent_chat_prompt_after_readiness(
        &self,
        message_id: &str,
        expected_run_id: &AgentChatRunId,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        prompt_dispatch_readiness::release(self, message_id, expected_run_id, host_epoch)
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
        transition(
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
        transition(
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
        transition(
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
        transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "launching",
            "pending",
            false,
        )
    }

    fn mark_agent_chat_prompt_unprovable(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        transition(
            self,
            message_id,
            coordinator_id,
            host_epoch,
            "launching",
            "unprovable",
            true,
        )
    }

    fn settle_agent_chat_prompt_dispatch(
        &self,
        message_id: &str,
        coordinator_id: &str,
        host_epoch: HostEpoch,
    ) -> Result<(), LedgerError> {
        transition(
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
        recover(self, host_epoch)
    }
}

fn claim(
    ledger: &SqliteLedger,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    provider: AgentChatProvider,
) -> Result<Option<AgentChatPromptSaved>, LedgerError> {
    helpers::valid_owner(coordinator_id)?;
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let message_id = transaction
        .query_row(
            "SELECT d.message_id FROM agent_chat_prompt_dispatches d JOIN conversation_messages m ON m.message_id = d.message_id JOIN agent_chat_run_selections s ON s.run_id = m.run_id WHERE s.provider = ?1 AND d.state = 'pending' AND m.run_id = (SELECT current.run_id FROM runs current JOIN agent_chat_run_selections selected ON selected.run_id = current.run_id WHERE current.conversation_id = m.conversation_id ORDER BY current.rowid DESC LIMIT 1) ORDER BY d.created_rowid LIMIT 1",
            params![provider_name(provider)],
            |row| row.get::<_, String>(0),
        )
        .optional()
        .map_err(storage_error)?;
    let Some(message_id) = message_id else {
        return Ok(None);
    };
    transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'claimed', coordinator_id = ?1, host_epoch = ?2 WHERE message_id = ?3 AND state = 'pending'",
        params![coordinator_id, host_epoch.0, message_id],
    ).map_err(storage_error)?;
    let saved = helpers::saved(&transaction, &message_id)?;
    transaction.commit().map_err(storage_error)?;
    Ok(Some(saved))
}

const fn provider_name(provider: AgentChatProvider) -> &'static str {
    match provider {
        AgentChatProvider::Claude => "claude",
        AgentChatProvider::Codex => "codex",
        AgentChatProvider::Claurst => "claurst",
    }
}

fn transition(
    ledger: &SqliteLedger,
    message_id: &str,
    coordinator_id: &str,
    host_epoch: HostEpoch,
    expected: &str,
    state: &str,
    retain_owner: bool,
) -> Result<(), LedgerError> {
    helpers::valid_owner(coordinator_id)?;
    if message_id.trim().is_empty() {
        return Err(LedgerError::Invariant(
            "agent chat dispatch message is invalid".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    let changed = if retain_owner {
        transaction.execute(
            "UPDATE agent_chat_prompt_dispatches SET state = ?1 WHERE message_id = ?2 AND state = ?3 AND coordinator_id = ?4 AND host_epoch = ?5",
            params![state, message_id, expected, coordinator_id, host_epoch.0],
        )
    } else {
        transaction.execute(
            "UPDATE agent_chat_prompt_dispatches SET state = ?1, coordinator_id = NULL, host_epoch = NULL WHERE message_id = ?2 AND state = ?3 AND coordinator_id = ?4 AND host_epoch = ?5",
            params![state, message_id, expected, coordinator_id, host_epoch.0],
        )
    }.map_err(storage_error)?;
    if changed != 1 {
        return Err(LedgerError::Invariant(
            "agent chat dispatch is not owned by this coordinator".into(),
        ));
    }
    transaction.commit().map_err(storage_error)
}

fn recover(ledger: &SqliteLedger, host_epoch: HostEpoch) -> Result<(), LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_open(&transaction, host_epoch)?;
    transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'pending', coordinator_id = NULL, host_epoch = NULL WHERE state = 'claimed' AND host_epoch < ?1",
        [host_epoch.0],
    ).map_err(storage_error)?;
    transaction.execute(
        "UPDATE agent_chat_prompt_dispatches SET state = 'unprovable' WHERE state IN ('launching', 'started') AND host_epoch < ?1",
        [host_epoch.0],
    ).map_err(storage_error)?;
    transaction.commit().map_err(storage_error)
}

#[path = "prompt_dispatch_helpers.rs"]
mod helpers;

#[path = "prompt_dispatch_terminal.rs"]
mod terminal;

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
