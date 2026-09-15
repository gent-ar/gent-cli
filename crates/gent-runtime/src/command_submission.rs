use gent_ports::{Ledger, ReceiptClaim};
use gent_types::{Command, Event, Receipt, ReceiptStatus};

use crate::{Coordinator, RuntimeError};

impl<L: Ledger> Coordinator<L> {
    /// # Errors
    /// Returns an error when the host fence rejects ingress or durable persistence fails.
    pub fn submit(&self, command: &Command) -> Result<Receipt, RuntimeError> {
        let accepted = Event {
            cursor: 0,
            event_id: format!("{}:accepted", command.receipt_id.0),
            receipt_id: command.receipt_id.clone(),
            host_epoch: command.host_epoch,
            kind: "commandAccepted".into(),
            payload: command.payload.clone(),
        };
        let receipt = match self.ledger.claim_command(command, &accepted)? {
            ReceiptClaim::Existing(receipt) | ReceiptClaim::Accepted(receipt) => receipt,
        };
        let status = terminal_status(&command.kind);
        let terminal = Event {
            cursor: 0,
            event_id: format!("{}:terminal", receipt.receipt_id.0),
            receipt_id: receipt.receipt_id.clone(),
            host_epoch: receipt.host_epoch,
            kind: terminal_kind(&status).into(),
            payload: serde_json::json!({ "status": status }),
        };
        Ok(self
            .ledger
            .settle_receipt(&receipt.idempotency_key, status, &terminal)?)
    }
}

fn terminal_status(kind: &str) -> ReceiptStatus {
    if kind == "decision" {
        ReceiptStatus::Unprovable
    } else {
        ReceiptStatus::Settled
    }
}

fn terminal_kind(status: &ReceiptStatus) -> &'static str {
    if *status == ReceiptStatus::Unprovable {
        "decisionUnprovable"
    } else {
        "commandSettled"
    }
}
