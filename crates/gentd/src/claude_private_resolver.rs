//! Provider-scoped resolver for the dormant private Claude authority seam.

use gent_ports::{PublicProviderResolver, PublicProviderRunError};
use gent_types::RunVersionLock;

/// Rejects every provider except Claude before durable locked resolution can run.
///
/// The authority composition wraps a ledger-backed resolver, so it can use only Gent's verified
/// private installation and never discovers an executable through `PATH`.
#[derive(Debug)]
pub(crate) struct ClaudeOnlyResolver<R> {
    inner: R,
}

impl<R> ClaudeOnlyResolver<R> {
    /// Binds an inert provider-scoped resolver without performing resolution.
    #[must_use]
    pub(crate) const fn new(inner: R) -> Self {
        Self { inner }
    }
}

impl<R: PublicProviderResolver> PublicProviderResolver for ClaudeOnlyResolver<R> {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        if provider != "claude" {
            return Err(PublicProviderRunError::CompatibilityDenied);
        }
        self.inner.resolve(provider)
    }
}
