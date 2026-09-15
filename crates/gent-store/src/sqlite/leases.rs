use gent_ports::{
    IngressMode, LeaseClaim, LedgerError, RunLease, RunLeaseClaim, RunRecord, RunSessionBinding,
    WorktreeLease,
};
use gent_types::{Event, HostEpoch, ReceiptId, RunVersionLock};
use rusqlite::TransactionBehavior;

use super::queries::{append_event, find_lease, insert_lease, replace_lease};
use super::queries::{
    find_run, find_run_lease, find_run_session_binding, find_run_version_lock, insert_run_lease,
    replace_run_lease, save_run_version_lock, storage_error,
};
use super::{SqliteLedger, epoch::require_epoch, host_ingress};

pub(super) fn claim_worktree(
    ledger: &SqliteLedger,
    requested: &WorktreeLease,
) -> Result<LeaseClaim, LedgerError> {
    let mut connection = ledger.lock()?;
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
    let claim = match find_run_lease(&transaction, &lease.run_id)? {
        None => {
            insert_run_lease(&transaction, lease)?;
            RunLeaseClaim::Acquired(lease.clone())
        }
        Some(existing) if existing == *lease => RunLeaseClaim::Acquired(existing),
        Some(existing) if existing.host_epoch == ingress.epoch => {
            return Ok(RunLeaseClaim::Contended(existing));
        }
        Some(previous) => {
            replace_run_lease(&transaction, lease)?;
            RunLeaseClaim::Recovered {
                previous,
                current: lease.clone(),
            }
        }
    };
    match find_run_version_lock(&transaction, &lease.run_id)? {
        None => save_run_version_lock(&transaction, &lease.run_id, lock)?,
        Some(previous) if previous != *lock => {
            rebind_run_version_lock(&transaction, lease, &previous, lock)?;
        }
        Some(_) => {}
    }
    transaction.commit().map_err(storage_error)?;
    Ok(claim)
}

fn rebind_run_version_lock(
    transaction: &rusqlite::Transaction<'_>,
    lease: &RunLease,
    previous: &RunVersionLock,
    current: &RunVersionLock,
) -> Result<(), LedgerError> {
    transaction
        .execute(
            "UPDATE run_version_locks SET canonical_path = ?2, file_identity = ?3, digest_sha256 = ?4, version = ?5, compatibility_entry = ?6 WHERE run_id = ?1",
            rusqlite::params![lease.run_id, current.canonical_path, current.file_identity, current.digest_sha256, current.version, current.compatibility_entry],
        )
        .map_err(storage_error)?;
    append_event(
        transaction,
        &Event {
            cursor: 0,
            event_id: format!(
                "run-executable-rebound:{}:{}:{}",
                lease.run_id, previous.file_identity, current.file_identity
            ),
            receipt_id: ReceiptId(format!("run-executable-rebound:{}", lease.run_id)),
            host_epoch: lease.host_epoch,
            kind: "runExecutableRebound".into(),
            payload: serde_json::json!({
                "runId": lease.run_id,
                "previous": previous,
                "current": current,
            }),
        },
    )
    .map(|_| ())
}

pub(super) fn retire_run_session_binding(
    ledger: &SqliteLedger,
    binding: &RunSessionBinding,
    host_epoch: HostEpoch,
) -> Result<(), LedgerError> {
    let mut connection = ledger.lock()?;
    let transaction = connection
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .map_err(storage_error)?;
    require_epoch(host_epoch, host_ingress(&transaction)?.epoch)?;
    match find_run_session_binding(&transaction, &binding.run_id)? {
        None => return Ok(()),
        Some(current) if current != *binding => {
            return Err(LedgerError::Invariant(
                "a different provider session is bound to the run".into(),
            ));
        }
        Some(_) => {}
    }
    transaction
        .execute(
            "DELETE FROM run_session_bindings WHERE run_id = ?1",
            [&binding.run_id],
        )
        .map_err(storage_error)?;
    append_event(
        &transaction,
        &Event {
            cursor: 0,
            event_id: format!(
                "run-session-retired:{}:{}",
                binding.run_id, binding.provider_session_id
            ),
            receipt_id: ReceiptId(format!("run-session-retired:{}", binding.run_id)),
            host_epoch,
            kind: "runSessionRetired".into(),
            payload: serde_json::json!({
                "runId": binding.run_id,
                "providerSessionId": binding.provider_session_id,
            }),
        },
    )?;
    transaction.commit().map_err(storage_error)
}
