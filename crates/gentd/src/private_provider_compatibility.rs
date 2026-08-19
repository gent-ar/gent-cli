//! Typed signed-compatibility binding for a privately observed provider lock.
//!
//! Provisioning owns the effect and this narrow port owns the decision whether its observed
//! executable can become durable authority. Neither knows the other's storage or process details.

use gent_types::RunVersionLock;

/// Binds one observed provider lock to current signed compatibility evidence.
pub(crate) trait ProvisionedProviderCompatibility: Clone + Send + Sync {
    /// Returns the signed-entry-bound lock, or denies it at the supplied authority time.
    fn bind(
        &self,
        observed: RunVersionLock,
        now_unix_seconds: u64,
    ) -> Result<RunVersionLock, String>;
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct TestProvisionedProviderCompatibility;

#[cfg(test)]
impl ProvisionedProviderCompatibility for TestProvisionedProviderCompatibility {
    fn bind(&self, observed: RunVersionLock, _: u64) -> Result<RunVersionLock, String> {
        Ok(observed)
    }
}
