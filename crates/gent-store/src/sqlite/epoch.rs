use super::SqliteLedger;
use super::queries::{host_ingress, storage_error};
use gent_ports::{HostIngress, IngressMode, LedgerError};
use gent_types::HostEpoch;

pub(super) fn require_epoch(command: HostEpoch, active: HostEpoch) -> Result<(), LedgerError> {
    if command == active {
        Ok(())
    } else {
        Err(LedgerError::StaleEpoch { command, active })
    }
}

pub(super) fn close_ingress(
    ledger: &SqliteLedger,
    epoch: HostEpoch,
) -> Result<HostIngress, LedgerError> {
    let mut connection = ledger.lock()?;
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

pub(super) fn fence_and_open(
    ledger: &SqliteLedger,
    epoch: HostEpoch,
) -> Result<HostIngress, LedgerError> {
    let mut connection = ledger.lock()?;
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
