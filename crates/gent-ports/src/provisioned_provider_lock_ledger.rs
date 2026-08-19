//! Durable provenance for Gent-owned public-provider installation locks.

use gent_types::{Event, ProvisionedProviderInstallation, Receipt, ReceiptStatus};

use crate::LedgerError;

/// Reads verified public-provider installations without permitting an authority effect.
///
/// Resolvers use this narrow port so executable selection cannot mutate provisioning state.
pub trait ProvisionedProviderLockReader: Send + Sync {
    /// Returns the most recently verified lock for one public provider, if it exists.
    ///
    /// # Errors
    /// Returns an error when durable storage is unavailable.
    fn find_provisioned_provider_installation(
        &self,
        provider: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError>;
}

/// Persists and reads verified public-provider installations without authorizing effects.
pub trait ProvisionedProviderLockLedger: Send + Sync {
    /// Atomically records one verified installation, when present, and terminally settles the
    /// exact accepted provisioning receipt.
    ///
    /// # Errors
    /// Returns an error when the receipt is not exactly accepted, the terminal event differs from
    /// that receipt, the status and installation disagree, a lock conflicts with a prior record,
    /// or durable storage is unavailable.
    fn settle_provisioned_provider_provision(
        &self,
        receipt: &Receipt,
        installation: Option<&ProvisionedProviderInstallation>,
        status: ReceiptStatus,
        terminal: &Event,
    ) -> Result<Receipt, LedgerError>;

    /// Returns the most recently verified lock for one public provider, if it exists.
    ///
    /// # Errors
    /// Returns an error when durable storage is unavailable.
    fn find_provisioned_provider_installation(
        &self,
        provider: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError>;
}

impl<T: ProvisionedProviderLockLedger + ?Sized> ProvisionedProviderLockReader for T {
    fn find_provisioned_provider_installation(
        &self,
        provider: &str,
    ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
        ProvisionedProviderLockLedger::find_provisioned_provider_installation(self, provider)
    }
}
