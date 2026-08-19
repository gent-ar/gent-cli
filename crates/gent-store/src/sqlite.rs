use gent_ports::{
    DecisionClaim, DecisionPhaseUpdate, HostIngress, IngressMode, LeaseClaim, Ledger, LedgerError,
    ReceiptClaim, RunLease, RunLeaseClaim, RunRecord, RunSessionBinding, WorktreeLease,
};
use gent_types::{
    Command, DecisionCommand, DecisionSettlement, DecisionSettlementPhase, Event, EventPage,
    HostEpoch, Receipt, ReceiptStatus, RunVersionLock,
};
use rusqlite::{Connection, params};
use std::sync::{Arc, Mutex};
mod agent_chat_compaction_ledger;
mod agent_chat_ledger;
mod agent_chat_read_ledger;
mod agent_chat_run_context_ledger;
mod agent_chat_terminal_settlement;
#[cfg(test)]
mod agent_chat_terminal_settlement_tests;
mod agent_chat_workspace_ledger;
mod attachment_ledger;
mod connection;
mod conversation_activity_ledger;
mod conversation_artifacts;
mod conversation_ledger;
mod conversation_prompt_ledger;
mod conversation_prompts;
mod conversations;
mod decisions;
mod epoch;
mod event_pages;
mod fresh_schema;
mod git_operation_ledger;
mod git_operations;
mod goal_ledger;
#[cfg(test)]
mod goal_ledger_tests;
mod leases;
mod mcp_connector_ledger;
mod mcp_connectors;
mod normalized_session_activity;
mod normalized_session_ledger;
#[cfg(test)]
mod normalized_session_ledger_tests;
mod orchestration_facts;
mod orchestration_ledger;
mod policies;
mod policy_ledger;
mod provision_receipts;
mod provisioned_provider_locks;
mod queries;
mod reviewed_plan_values;
mod reviewed_plans;
mod run_checkpoint_ledger;
mod run_checkpoints;
mod run_lifecycle_facts;
mod runs;
mod runtime_update_journal;
mod tool_source_ledger;
mod tool_sources;
mod transcript_ledger;
mod turn_follow_ledger;
mod workspace_ledger;
mod workspaces;
use epoch::require_epoch;
use queries::{
    append_event, encode_status, find_event, find_lease, find_receipt, find_run,
    find_run_session_binding, find_run_version_lock, host_ingress, insert_lease, insert_receipt,
    receipt_matches_command, replace_lease, save_run_session_binding, save_run_version_lock,
    storage_error,
};
#[derive(Clone, Debug)]
pub struct SqliteLedger {
    connection: Arc<Mutex<Connection>>,
}
impl Ledger for SqliteLedger {
    fn host_ingress(&self) -> Result<HostIngress, LedgerError> {
        host_ingress(&*self.lock()?)
    }
    fn close_ingress(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let state = host_ingress(&transaction)?;
        require_epoch(epoch, state.epoch)?;
        transaction
            .execute(
                "UPDATE host_state SET ingress = 'closed' WHERE singleton = 1",
                [],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(HostIngress {
            epoch,
            mode: IngressMode::Closed,
        })
    }
    fn fence_and_open(&self, epoch: HostEpoch) -> Result<HostIngress, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let state = host_ingress(&transaction)?;
        require_epoch(epoch, state.epoch)?;
        if state.mode != IngressMode::Closed {
            return Err(LedgerError::Invariant(
                "ingress must be closed before fencing".into(),
            ));
        }
        let next = HostEpoch(
            epoch
                .0
                .checked_add(1)
                .ok_or_else(|| LedgerError::Invariant("host epoch overflow".into()))?,
        );
        transaction
            .execute(
                "UPDATE host_state SET epoch = ?1, ingress = 'open' WHERE singleton = 1",
                [next.0],
            )
            .map_err(storage_error)?;
        transaction.commit().map_err(storage_error)?;
        Ok(HostIngress {
            epoch: next,
            mode: IngressMode::Open,
        })
    }
    fn claim_command(
        &self,
        command: &Command,
        accepted: &Event,
    ) -> Result<ReceiptClaim, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let state = host_ingress(&transaction)?;
        require_epoch(command.host_epoch, state.epoch)?;
        if state.mode == IngressMode::Closed {
            return Err(LedgerError::IngressClosed { epoch: state.epoch });
        }
        if let Some(receipt) = find_receipt(&transaction, &command.idempotency_key)? {
            if !receipt_matches_command(&transaction, command)? {
                return Err(LedgerError::Invariant(
                    "idempotency key is bound to a different command".into(),
                ));
            }
            return Ok(ReceiptClaim::Existing(receipt));
        }
        let receipt = Receipt {
            receipt_id: command.receipt_id.clone(),
            idempotency_key: command.idempotency_key.clone(),
            status: ReceiptStatus::Accepted,
            host_epoch: state.epoch,
        };
        insert_receipt(&transaction, &receipt, command)?;
        append_event(&transaction, accepted)?;
        transaction.commit().map_err(storage_error)?;
        Ok(ReceiptClaim::Accepted(receipt))
    }
    fn settle_receipt(
        &self,
        key: &str,
        status: ReceiptStatus,
        terminal: &Event,
    ) -> Result<Receipt, LedgerError> {
        if status == ReceiptStatus::Accepted {
            return Err(LedgerError::Invariant(
                "receipt settlement must be terminal".into(),
            ));
        }
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let mut receipt = find_receipt(&transaction, key)?
            .ok_or_else(|| LedgerError::Invariant("cannot settle an unknown receipt".into()))?;
        if receipt.status != ReceiptStatus::Accepted {
            if receipt.status == status {
                return Ok(receipt);
            }
            return Err(LedgerError::Invariant(
                "receipt already has a different terminal state".into(),
            ));
        }
        transaction
            .execute(
                "UPDATE receipts SET status = ?1 WHERE idempotency_key = ?2",
                params![encode_status(&status), key],
            )
            .map_err(storage_error)?;
        append_event(&transaction, terminal)?;
        transaction.commit().map_err(storage_error)?;
        receipt.status = status;
        Ok(receipt)
    }
    fn claim_decision(&self, command: &DecisionCommand) -> Result<DecisionClaim, LedgerError> {
        decisions::claim(self, command)
    }
    fn find_decision(&self, decision_id: &str) -> Result<Option<DecisionSettlement>, LedgerError> {
        decisions::find(self, decision_id)
    }
    fn replace_decision_phase(
        &self,
        decision_id: &str,
        expected: &DecisionSettlementPhase,
        next: &DecisionSettlementPhase,
    ) -> Result<DecisionPhaseUpdate, LedgerError> {
        decisions::replace_phase(self, decision_id, expected, next)
    }
    fn append_event(&self, event: &Event) -> Result<Event, LedgerError> {
        append_event(&*self.lock()?, event)
    }
    fn find_event(&self, event_id: &str) -> Result<Option<Event>, LedgerError> {
        find_event(&*self.lock()?, event_id)
    }
    fn read_event_page(&self, after_cursor: u64, limit: usize) -> Result<EventPage, LedgerError> {
        event_pages::read(&*self.lock()?, after_cursor, limit)
    }
    fn create_run(&self, run: &RunRecord) -> Result<(), LedgerError> {
        runs::create(self, run)
    }
    fn reserve_run_start(
        &self,
        run: &RunRecord,
        lock: &RunVersionLock,
        lease: &RunLease,
    ) -> Result<(), LedgerError> {
        leases::reserve_run_start(self, run, lock, lease)
    }
    fn activate_existing_run_start(
        &self,
        lock: &RunVersionLock,
        lease: &RunLease,
    ) -> Result<RunLeaseClaim, LedgerError> {
        leases::activate_existing_run_start(self, lock, lease)
    }
    fn find_run(&self, run_id: &str) -> Result<Option<RunRecord>, LedgerError> {
        find_run(&*self.lock()?, run_id)
    }
    fn save_run_version_lock(
        &self,
        run_id: &str,
        lock: &RunVersionLock,
    ) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        save_run_version_lock(&connection, run_id, lock)
    }
    fn find_run_version_lock(&self, run_id: &str) -> Result<Option<RunVersionLock>, LedgerError> {
        let connection = self.lock()?;
        find_run_version_lock(&connection, run_id)
    }
    fn save_run_session_binding(&self, binding: &RunSessionBinding) -> Result<(), LedgerError> {
        let connection = self.lock()?;
        if find_run(&connection, &binding.run_id)?.is_none() {
            return Err(LedgerError::Invariant("run does not exist".into()));
        }
        save_run_session_binding(&connection, binding)
    }
    fn find_run_session_binding(
        &self,
        run_id: &str,
    ) -> Result<Option<RunSessionBinding>, LedgerError> {
        let connection = self.lock()?;
        find_run_session_binding(&connection, run_id)
    }
    fn claim_run_lease(&self, requested: &RunLease) -> Result<RunLeaseClaim, LedgerError> {
        leases::claim_run(self, requested)
    }
    fn find_run_lease(&self, run_id: &str) -> Result<Option<RunLease>, LedgerError> {
        let connection = self.lock()?;
        queries::find_run_lease(&connection, run_id)
    }
    fn claim_worktree_lease(&self, requested: &WorktreeLease) -> Result<LeaseClaim, LedgerError> {
        let mut connection = self.lock()?;
        let transaction = connection.transaction().map_err(storage_error)?;
        let active = host_ingress(&transaction)?.epoch;
        require_epoch(requested.host_epoch, active)?;
        if find_run(&transaction, &requested.run_id)?.is_none() {
            return Err(LedgerError::Invariant("lease run does not exist".into()));
        }
        let result = match find_lease(&transaction, &requested.worktree_id)? {
            None => {
                insert_lease(&transaction, requested)?;
                LeaseClaim::Acquired(requested.clone())
            }
            Some(existing) if existing.host_epoch == active => LeaseClaim::Contended(existing),
            Some(previous) => {
                replace_lease(&transaction, requested)?;
                LeaseClaim::Recovered {
                    previous,
                    current: requested.clone(),
                }
            }
        };
        transaction.commit().map_err(storage_error)?;
        Ok(result)
    }
    fn find_worktree_lease(&self, id: &str) -> Result<Option<WorktreeLease>, LedgerError> {
        let connection = self.lock()?;
        find_lease(&connection, id)
    }
}
