//! Ports implemented by infrastructure or private integrations.

use async_trait::async_trait;
use gent_types::{Command, Event, HostEpoch, ProviderEvent, Receipt, ReceiptStatus};

#[derive(Debug, thiserror::Error)]
pub enum PortError {
    #[error("provider bridge failure: {0}")]
    Provider(String),
}

/// Private Claurst implementations receive only opaque references through this port.
#[async_trait]
pub trait ExternalProviderBridge: Send + Sync {
    async fn submit(&self, opaque_session: &str, command: Command) -> Result<(), PortError>;
    async fn next_event(&self, opaque_session: &str) -> Result<Option<ProviderEvent>, PortError>;
}

#[async_trait]
pub trait ProviderDriver: Send + Sync {
    async fn submit(&self, command: Command) -> Result<(), PortError>;
}

#[derive(Debug, thiserror::Error)]
pub enum LedgerError {
    #[error("ledger failure: {0}")]
    Storage(String),
}

/// Persistence boundary used by the coordinator. Implementations own durability, not policy.
pub trait Ledger: Send + Sync {
    /// Returns the active epoch.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn current_epoch(&self) -> Result<HostEpoch, LedgerError>;
    /// Looks up a previously accepted idempotency key.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn find_receipt(&self, idempotency_key: &str) -> Result<Option<Receipt>, LedgerError>;
    /// Commits a receipt before the command outcome is returned.
    ///
    /// # Errors
    /// Returns an error when the receipt cannot be made durable.
    fn record_receipt(&self, receipt: &Receipt) -> Result<(), LedgerError>;
    /// Changes a receipt to its terminal state.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be updated.
    fn update_receipt_status(
        &self,
        idempotency_key: &str,
        status: ReceiptStatus,
    ) -> Result<(), LedgerError>;
    /// Appends a cursor-ordered event.
    ///
    /// # Errors
    /// Returns an error when the event cannot be made durable.
    fn append_event(&self, event: &Event) -> Result<Event, LedgerError>;
    /// Reads all events strictly after a cursor.
    ///
    /// # Errors
    /// Returns an error when durable state cannot be read.
    fn events_after(&self, cursor: u64) -> Result<Vec<Event>, LedgerError>;
}
