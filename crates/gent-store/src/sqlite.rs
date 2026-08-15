use gent_ports::{
    DecisionClaim, DecisionPhaseUpdate, HostIngress, IngressMode, LeaseClaim, Ledger, LedgerError,
    ReceiptClaim, RunLease, RunLeaseClaim, RunRecord, RunSessionBinding, WorktreeLease,
};
use gent_types::{
    Command, DecisionCommand, DecisionSettlement, DecisionSettlementPhase, Event, EventResume,
    EventSnapshot, HostEpoch, Receipt, ReceiptStatus, RunVersionLock,
};
use rusqlite::{Connection, params};
use std::{path::Path, sync::Arc, sync::Mutex, time::Duration};
mod attachment_ledger;
mod automation_execution_ledger;
mod automation_executions;
mod capability_catalog;
mod conversation_artifacts;
mod conversation_ledger;
mod conversations;
mod decisions;
mod epoch;
mod git_operation_ledger;
mod git_operations;
mod leases;
mod migrations;
mod policies;
mod policy_ledger;
mod projections;
mod queries;
mod run_checkpoint_ledger;
mod run_checkpoints;
mod runs;
mod snapshots;
mod tool_source_ledger;
mod tool_sources;
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
impl SqliteLedger {
    /// Opens or creates a durable ledger at `path`.
    /// # Errors
    /// Returns an error when `SQLite` cannot open or migrate the database.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, LedgerError> {
        Self::from_connection(Connection::open(path).map_err(storage_error)?)
    }
    /// Creates an isolated in-memory ledger for deterministic tests.
    /// # Errors
    /// Returns an error when `SQLite` cannot initialize the database.
    pub fn in_memory() -> Result<Self, LedgerError> {
        Self::from_connection(Connection::open_in_memory().map_err(storage_error)?)
    }
    fn from_connection(mut connection: Connection) -> Result<Self, LedgerError> {
        connection
            .busy_timeout(Duration::from_secs(3))
            .map_err(storage_error)?;
        connection
            .execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")
            .map_err(storage_error)?;
        migrations::apply(&mut connection)?;
        Ok(Self {
            connection: Arc::new(Mutex::new(connection)),
        })
    }
    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, LedgerError> {
        self.connection
            .lock()
            .map_err(|error| LedgerError::Storage(error.to_string()))
    }
}
impl Ledger for SqliteLedger {
    fn host_ingress(&self) -> Result<HostIngress, LedgerError> {
        let connection = self.lock()?;
        host_ingress(&connection)
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
    fn resume_events(&self, cursor: u64) -> Result<EventResume, LedgerError> {
        let connection = self.lock()?;
        snapshots::resume(&connection, cursor)
    }
    fn compact_events(&self, snapshot: &EventSnapshot) -> Result<(), LedgerError> {
        let mut connection = self.lock()?;
        snapshots::compact(&mut connection, snapshot)
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
    fn find_run(&self, run_id: &str) -> Result<Option<RunRecord>, LedgerError> {
        let connection = self.lock()?;
        find_run(&connection, run_id)
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
