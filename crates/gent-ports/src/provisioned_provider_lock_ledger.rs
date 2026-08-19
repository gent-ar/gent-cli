//! Durable provenance for Gent-owned public-provider installation locks.

use gent_types::{
    Command, Event, ProviderPromptProvisionCommandBinding, ProvisionedProviderInstallation,
    Receipt, ReceiptStatus,
};

use crate::{LedgerError, ReceiptClaim};

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

/// Atomically settles one verified prompt-scoped provision and releases that exact held prompt.
///
/// This is distinct from generic dependency actions: its immutable command fingerprint binds the
/// prompt receipt, conversation, current run, provider, action, consent, and reviewed digest.
pub trait PrivateProviderPromptProvisionLedger: Send + Sync {
    /// Atomically claims one command and reserves its exact held prompt before an npm effect.
    ///
    /// A newly accepted receipt always means the dispatch has moved to `provisioning`; recovery
    /// can therefore terminally mark it unprovable without guessing whether an effect began.
    ///
    /// # Errors
    /// Returns when ingress, command, accepted event, current prompt, or dispatch state differs.
    fn claim_and_reserve_verified_provider_prompt_provision(
        &self,
        command: &Command,
        accepted: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<ReceiptClaim, LedgerError>;

    /// Reserves one exact held prompt before its daemon-owned private npm effect.
    ///
    /// This persistent admission fences the current selected run and changes only the bound
    /// dispatch from `awaiting_readiness` to `provisioning`. A selection switch cannot overtake
    /// the external effect after this returns successfully.
    ///
    /// # Errors
    /// Returns when the exact accepted command, epoch, prompt receipt, provider selection, or
    /// held dispatch differs, leaving the dispatch unchanged.
    fn reserve_verified_provider_prompt_provision(
        &self,
        command: &Command,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<(), LedgerError>;

    /// Terminally rejects one current held prompt confirmation without reserving its effect.
    ///
    /// A consent refusal or stale review receives an idempotent receipt while leaving the held
    /// prompt in `awaiting_readiness`, so a fresh explicit confirmation may be attempted later.
    ///
    /// # Errors
    /// Returns when the exact command, receipt, current prompt binding, or terminal event differs.
    fn settle_rejected_provider_prompt_provision(
        &self,
        command: &Command,
        receipt: &Receipt,
        terminal: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<Receipt, LedgerError>;

    /// Terminally marks one reserved prompt provision unprovable without releasing its prompt.
    ///
    /// This is used after an accepted receipt is recovered or an external effect becomes
    /// ambiguous. It prevents a replay while preserving the prompt as a durable failure.
    ///
    /// # Errors
    /// Returns when the receipt, command, terminal event, or reserved prompt binding differs.
    fn settle_unprovable_provider_prompt_provision(
        &self,
        command: &Command,
        receipt: &Receipt,
        terminal: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<Receipt, LedgerError>;

    /// Writes the lock, receipt/event, and one reserved-prompt release in the same transaction.
    ///
    /// # Errors
    /// Returns when any prompt/run/provider/receipt/command fence differs, leaving every fact
    /// unmodified.
    fn settle_verified_provider_prompt_provision(
        &self,
        command: &Command,
        receipt: &Receipt,
        installation: &ProvisionedProviderInstallation,
        terminal: &Event,
        binding: &ProviderPromptProvisionCommandBinding,
    ) -> Result<Receipt, LedgerError>;
}
