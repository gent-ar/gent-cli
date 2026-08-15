//! Host-epoch fencing shared by `SQLite` ledger operations.

use gent_ports::LedgerError;
use gent_types::HostEpoch;

pub(super) fn require_epoch(command: HostEpoch, active: HostEpoch) -> Result<(), LedgerError> {
    if command == active {
        Ok(())
    } else {
        Err(LedgerError::StaleEpoch { command, active })
    }
}
