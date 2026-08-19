//! Durable reservation for an explicit dependency effect.
//!
//! This contains the common idempotency rule, but intentionally owns no effect or terminal
//! settlement. Effect-specific authorities must settle their own verified outcomes.

use std::sync::{Arc, Mutex};

use gent_ports::{Ledger, ReceiptClaim};
use gent_protocol::DependencyActionRequest;
use gent_types::{Event, Receipt, ReceiptStatus};

use crate::{RuntimeError, dependency_action_command};

/// One durable dependency-action receipt reservation.
#[derive(Clone, Debug)]
pub struct DependencyActionReceiptReservation<L> {
    ledger: L,
    serial: Arc<Mutex<()>>,
}

/// The durable state found while reserving one dependency action.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DependencyActionReceiptClaim {
    /// This caller owns the newly accepted receipt and must settle it.
    Claimed(Receipt),
    /// A prior process may have run the effect; it must be settled unprovable, never replayed.
    AcceptedRecovery(Receipt),
    /// The effect already reached a durable terminal state.
    Terminal(Receipt),
}

impl<L> DependencyActionReceiptReservation<L> {
    /// Creates a serial reservation owner for one durable ledger.
    #[must_use]
    pub fn new(ledger: L) -> Self {
        Self {
            ledger,
            serial: Arc::new(Mutex::new(())),
        }
    }
}

impl<L: Ledger> DependencyActionReceiptReservation<L> {
    /// Claims the exact action command, without starting or settling an effect.
    ///
    /// # Errors
    /// Returns an error when the durable claim cannot be recorded.
    pub fn reserve(
        &self,
        request: &DependencyActionRequest,
    ) -> Result<DependencyActionReceiptClaim, RuntimeError> {
        let _serial = self
            .serial
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let command = dependency_action_command(request);
        match self
            .ledger
            .claim_command(&command, &accepted_event(&command))?
        {
            ReceiptClaim::Accepted(receipt) => Ok(DependencyActionReceiptClaim::Claimed(receipt)),
            ReceiptClaim::Existing(receipt) if receipt.status == ReceiptStatus::Accepted => {
                Ok(DependencyActionReceiptClaim::AcceptedRecovery(receipt))
            }
            ReceiptClaim::Existing(receipt) => Ok(DependencyActionReceiptClaim::Terminal(receipt)),
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

fn accepted_event(command: &gent_types::Command) -> Event {
    Event {
        cursor: 0,
        event_id: format!("{}:dependency-accepted", command.receipt_id.0),
        receipt_id: command.receipt_id.clone(),
        host_epoch: command.host_epoch,
        kind: "dependencyActionAccepted".into(),
        payload: command.payload.clone(),
    }
}
