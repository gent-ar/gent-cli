//! Coordinator implementation over the persistence and provider ports.

use gent_core::{CoreError, require_current_epoch};
use gent_ports::{Ledger, LedgerError};
use gent_types::{
    CapabilitySet, Command, Event, HostStatus, PROTOCOL_MAX, PROTOCOL_MIN, Receipt, ReceiptStatus,
};

#[derive(Clone, Debug)]
pub struct Coordinator<L> {
    ledger: L,
    capabilities: CapabilitySet,
}

#[derive(Debug, thiserror::Error)]
pub enum RuntimeError {
    #[error(transparent)]
    Core(#[from] CoreError),
    #[error(transparent)]
    Ledger(#[from] LedgerError),
}

impl<L: Ledger> Coordinator<L> {
    #[must_use]
    pub fn new(ledger: L, capabilities: CapabilitySet) -> Self {
        Self {
            ledger,
            capabilities,
        }
    }

    /// Returns the negotiated host state.
    ///
    /// # Errors
    /// Returns an error when the ledger cannot read the active epoch.
    pub fn status(&self) -> Result<HostStatus, RuntimeError> {
        Ok(HostStatus {
            host_epoch: self.ledger.current_epoch()?,
            protocol_min: PROTOCOL_MIN,
            protocol_max: PROTOCOL_MAX,
            capabilities: self.capabilities.clone(),
        })
    }

    /// Accepts one idempotent command and records terminal local state.
    ///
    /// # Errors
    /// Returns an error for stale epochs or failed durable operations.
    pub fn submit(&self, command: Command) -> Result<Receipt, RuntimeError> {
        if let Some(receipt) = self.ledger.find_receipt(&command.idempotency_key)? {
            return Ok(receipt);
        }
        let epoch = self.ledger.current_epoch()?;
        require_current_epoch(command.host_epoch, epoch)?;

        let mut receipt = Receipt {
            receipt_id: command.receipt_id.clone(),
            idempotency_key: command.idempotency_key.clone(),
            status: ReceiptStatus::Accepted,
            host_epoch: epoch,
        };
        // The receipt is committed before any outcome is reported or event is emitted.
        self.ledger.record_receipt(&receipt)?;
        self.ledger.append_event(&Event {
            cursor: 0,
            event_id: format!("{}:accepted", receipt.receipt_id.0),
            receipt_id: receipt.receipt_id.clone(),
            host_epoch: epoch,
            kind: "commandAccepted".into(),
            payload: command.payload,
        })?;

        receipt.status = if command.kind == "decision" {
            ReceiptStatus::Unprovable
        } else {
            ReceiptStatus::Settled
        };
        self.ledger
            .update_receipt_status(&receipt.idempotency_key, receipt.status.clone())?;
        self.ledger.append_event(&Event {
            cursor: 0,
            event_id: format!("{}:terminal", receipt.receipt_id.0),
            receipt_id: receipt.receipt_id.clone(),
            host_epoch: epoch,
            kind: match receipt.status {
                ReceiptStatus::Unprovable => "decisionUnprovable",
                _ => "commandSettled",
            }
            .into(),
            payload: serde_json::json!({ "status": receipt.status }),
        })?;
        Ok(receipt)
    }

    /// Resumes the durable event feed after `cursor`.
    ///
    /// # Errors
    /// Returns an error when the ledger cannot read events.
    pub fn events_after(&self, cursor: u64) -> Result<Vec<Event>, RuntimeError> {
        Ok(self.ledger.events_after(cursor)?)
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::Ledger;
    use gent_store::SqliteLedger;
    use gent_types::{CapabilitySet, Command, HostEpoch, ReceiptId, ReceiptStatus};
    use serde_json::json;

    use super::{Coordinator, RuntimeError};

    fn command(key: &str, epoch: u64, kind: &str) -> Command {
        Command {
            receipt_id: ReceiptId::new(),
            idempotency_key: key.into(),
            host_epoch: HostEpoch(epoch),
            kind: kind.into(),
            payload: json!({ "example": true }),
        }
    }

    #[test]
    fn idempotency_returns_the_original_receipt_without_duplicate_events() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
        let first = coordinator.submit(command("once", 1, "ping")).unwrap();
        let second = coordinator.submit(command("once", 1, "ping")).unwrap();
        assert_eq!(first, second);
        assert_eq!(ledger.events_after(0).unwrap().len(), 2);
    }

    #[test]
    fn stale_epoch_is_rejected_before_a_receipt_is_written() {
        let ledger = SqliteLedger::in_memory().unwrap();
        let coordinator = Coordinator::new(ledger.clone(), CapabilitySet::default());
        assert!(matches!(
            coordinator.submit(command("stale", 0, "ping")),
            Err(RuntimeError::Core(_))
        ));
        assert!(ledger.find_receipt("stale").unwrap().is_none());
    }

    #[test]
    fn decision_reaches_a_terminal_unprovable_state_without_a_provider_ack() {
        let coordinator =
            Coordinator::new(SqliteLedger::in_memory().unwrap(), CapabilitySet::default());
        assert_eq!(
            coordinator
                .submit(command("decision", 1, "decision"))
                .unwrap()
                .status,
            ReceiptStatus::Unprovable
        );
    }
}
