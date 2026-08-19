//! Narrow dependency-action receipt reservation over the generic daemon command reservation.

use gent_ports::Ledger;
use gent_protocol::DependencyActionRequest;
use gent_types::{Event, Receipt, ReceiptStatus};

use crate::{
    CommandReceiptClaim, CommandReceiptReservation, RuntimeError, dependency_action_command,
};

/// One durable dependency-action receipt reservation.
#[derive(Clone, Debug)]
pub struct DependencyActionReceiptReservation<L> {
    inner: CommandReceiptReservation<L>,
}

/// The durable state found while reserving one dependency action.
pub type DependencyActionReceiptClaim = CommandReceiptClaim;

impl<L> DependencyActionReceiptReservation<L> {
    /// Creates a serial reservation owner for one durable ledger.
    #[must_use]
    pub fn new(ledger: L) -> Self {
        Self {
            inner: CommandReceiptReservation::new(ledger),
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
        let command = dependency_action_command(request);
        self.inner.reserve(&command, "dependencyActionAccepted")
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
        self.inner.settle_receipt(idempotency_key, status, terminal)
    }
}
