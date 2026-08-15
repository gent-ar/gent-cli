use gent_ports::{LedgerError, RunLease, RunLeaseClaim};
use rusqlite::TransactionBehavior;

use super::queries::{
    find_run, find_run_lease, insert_run_lease, replace_run_lease, storage_error,
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
    let active = host_ingress(&transaction)?.epoch;
    require_epoch(requested.host_epoch, active)?;
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
