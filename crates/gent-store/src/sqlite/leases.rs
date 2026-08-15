use gent_ports::{IngressMode, LedgerError, RunLease, RunLeaseClaim, RunRecord};
use gent_types::RunVersionLock;
use rusqlite::TransactionBehavior;

use super::queries::{
    find_run, find_run_lease, insert_run_lease, replace_run_lease, save_run_version_lock,
    storage_error,
};
use super::{SqliteLedger, host_ingress, require_epoch};

/// Claims one durable run for a coordinator transactionally.
pub(super) fn claim_run(
    ledger: &SqliteLedger,
    requested: &RunLease,
) -> Result<RunLeaseClaim, LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    let active = ingress.epoch;
    require_epoch(requested.host_epoch, active)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed { epoch: active });
    }
    if find_run(&transaction, &requested.run_id)?.is_none() {
        return Err(LedgerError::Invariant(
            "run lease target does not exist".into(),
        ));
    }
    let result = match find_run_lease(&transaction, &requested.run_id)? {
        None => {
            insert_run_lease(&transaction, requested)?;
            RunLeaseClaim::Acquired(requested.clone())
        }
        Some(existing) if existing.host_epoch == active => RunLeaseClaim::Contended(existing),
        Some(previous) => {
            replace_run_lease(&transaction, requested)?;
            RunLeaseClaim::Recovered {
                previous,
                current: requested.clone(),
            }
        }
    };
    transaction.commit().map_err(storage_error)?;
    Ok(result)
}

/// Atomically records every durable precondition required before a public process can start.
pub(super) fn reserve_run_start(
    ledger: &SqliteLedger,
    run: &RunRecord,
    lock: &RunVersionLock,
    lease: &RunLease,
) -> Result<(), LedgerError> {
    if run.run_id != lease.run_id || run.provider != lock.provider {
        return Err(LedgerError::Invariant(
            "run, lock, and lease identities must agree".into(),
        ));
    }
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    let ingress = host_ingress(&transaction)?;
    require_epoch(lease.host_epoch, ingress.epoch)?;
    if ingress.mode == IngressMode::Closed {
        return Err(LedgerError::IngressClosed {
            epoch: ingress.epoch,
        });
    }
    if find_run(&transaction, &run.run_id)?.is_some() {
        return Err(LedgerError::Invariant("run already exists".into()));
    }
    if let Some(parent) = &run.parent_run_id {
        if find_run(&transaction, parent)?.is_none() {
            return Err(LedgerError::Invariant("run parent does not exist".into()));
        }
    }
    transaction
        .execute(
            "INSERT INTO runs (run_id, parent_run_id, provider) VALUES (?1, ?2, ?3)",
            rusqlite::params![run.run_id, run.parent_run_id, run.provider],
        )
        .map_err(storage_error)?;
    save_run_version_lock(&transaction, &run.run_id, lock)?;
    insert_run_lease(&transaction, lease)?;
    transaction.commit().map_err(storage_error)
}
