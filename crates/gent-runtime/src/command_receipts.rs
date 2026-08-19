//! Durable reservation for a daemon-built command effect.
//!
//! This common idempotency rule owns no effect or terminal settlement. Effect authorities build
//! their exact commands before calling it, then settle their own verified outcomes.

use std::sync::{Arc, Mutex};

use gent_ports::{Ledger, ReceiptClaim};
use gent_types::{Command, Event, Receipt, ReceiptStatus};

use crate::RuntimeError;

/// One serial durable receipt reservation owner.
#[derive(Clone, Debug)]
pub struct CommandReceiptReservation<L> {
    ledger: L,
    serial: Arc<Mutex<()>>,
}

/// The durable state found while reserving one command.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CommandReceiptClaim {
    /// This caller owns the newly accepted receipt and must settle it.
    Claimed(Receipt),
    /// A prior process may have run the effect; it must settle unprovable, never replay it.
    AcceptedRecovery(Receipt),
    /// The effect already reached a durable terminal state.
    Terminal(Receipt),
}

impl<L> CommandReceiptReservation<L> {
    /// Creates a serial reservation owner for one durable ledger.
    #[must_use]
    pub fn new(ledger: L) -> Self {
        Self {
            ledger,
            serial: Arc::new(Mutex::new(())),
        }
    }
}

impl<L: Ledger> CommandReceiptReservation<L> {
    /// Claims one exact daemon-built command without starting or settling its effect.
    ///
    /// # Errors
    /// Returns an error when the durable claim cannot be recorded.
    pub fn reserve(
        &self,
        command: &Command,
        accepted_kind: &str,
    ) -> Result<CommandReceiptClaim, RuntimeError> {
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        match self
            .ledger
            .claim_command(command, &accepted_event(command, accepted_kind))?
        {
            ReceiptClaim::Accepted(receipt) => Ok(CommandReceiptClaim::Claimed(receipt)),
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                Ok(CommandReceiptClaim::AcceptedRecovery(receipt))
            }
            ReceiptClaim::Existing(receipt) => Ok(CommandReceiptClaim::Terminal(receipt)),
        }
    }

    /// Settles a receipt reserved by this service.
    ///
    /// # Errors
    /// Returns an error when the terminal receipt cannot be recorded.
    pub fn settle_receipt(
        &self,
        idempotency_key: &str,
        status: ReceiptStatus,
        terminal: &Event,
    ) -> Result<Receipt, RuntimeError> {
        self.ledger
            .settle_receipt(idempotency_key, status, terminal)
            .map_err(RuntimeError::from)
    }
}

fn accepted_event(command: &Command, accepted_kind: &str) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: accepted_kind.into(),
        payload: command.payload.clone(),
    }
}
