use gent_ports::{IngressMode, LedgerError, RunLease, RunLeaseClaim, RunRecord};
use gent_types::RunVersionLock;
use rusqlite::TransactionBehavior;

use super::queries::{
    find_run, find_run_lease, find_run_version_lock, insert_run_lease, replace_run_lease,
    save_run_version_lock, storage_error,
};
use super::{SqliteLedger, epoch::require_epoch, host_ingress};

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
        // An unchanged coordinator may resume a one-shot provider session. Epoch validation
        // above still fences a stale daemon before this idempotent claim is accepted.
        Some(existing) if existing == *requested => RunLeaseClaim::Acquired(existing),
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
    let conversation_id = if let Some(parent) = &run.parent_run_id {
        find_run(&transaction, parent)?
            .ok_or_else(|| LedgerError::Invariant("run parent does not exist".into()))?;
        super::conversations::conversation_id_for_run(&transaction, parent)?
    } else {
        None
    };
    transaction
        .execute(
            "INSERT INTO runs (run_id, conversation_id, parent_run_id, provider) VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![run.run_id, conversation_id, run.parent_run_id, run.provider],
        )
        .map_err(storage_error)?;
    save_run_version_lock(&transaction, &run.run_id, lock)?;
    insert_run_lease(&transaction, lease)?;
    transaction.commit().map_err(storage_error)
}

/// Atomically attaches a lock and lease to a run created by a durable chat transaction.
pub(super) fn activate_existing_run_start(
    ledger: &SqliteLedger,
    lock: &RunVersionLock,
    lease: &RunLease,
) -> Result<RunLeaseClaim, LedgerError> {
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
    let run = find_run(&transaction, &lease.run_id)?
        .ok_or_else(|| LedgerError::Invariant("activation run does not exist".into()))?;
    if run.provider != lock.provider {
        return Err(LedgerError::Invariant(
            "activation run and executable provider must agree".into(),
        ));
    }
    match find_run_version_lock(&transaction, &lease.run_id)? {
        Some(existing) if existing != *lock => {
            return Err(LedgerError::Invariant(
                "activation cannot replace an immutable run lock".into(),
            ));
        }
        None => save_run_version_lock(&transaction, &lease.run_id, lock)?,
        Some(_) => {}
    }
    let claim = match find_run_lease(&transaction, &lease.run_id)? {
        None => {
            insert_run_lease(&transaction, lease)?;
            RunLeaseClaim::Acquired(lease.clone())
        }
        Some(existing) if existing == *lease => RunLeaseClaim::Acquired(existing),
        Some(existing) if existing.host_epoch == ingress.epoch => {
            RunLeaseClaim::Contended(existing)
        }
        Some(previous) => {
            replace_run_lease(&transaction, lease)?;
            RunLeaseClaim::Recovered {
                previous,
                current: lease.clone(),
            }
        }
    };
    transaction.commit().map_err(storage_error)?;
    Ok(claim)
}
