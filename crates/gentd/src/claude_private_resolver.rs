//! Provider-scoped resolver for the dormant private Claude authority seam.

use gent_ports::{PublicProviderResolver, PublicProviderRunError};
use gent_types::RunVersionLock;

use crate::provider_resolver::DaemonProviderResolver;

/// Rejects every provider except Claude before private executable discovery can run.
///
/// The wrapped resolver is configured with [`crate::provider_resolver::PrivatePrefixDiscovery`],
/// so an authority host can discover only Gent's locked private installation, never `PATH`.
#[derive(Debug)]
pub(crate) struct ClaudeOnlyResolver<D, P> {
    inner: DaemonProviderResolver<D, P>,
}

impl<D, P> ClaudeOnlyResolver<D, P> {
    /// Binds an inert private resolver without discovery or version probing.
    #[must_use]
    pub(crate) const fn new(inner: DaemonProviderResolver<D, P>) -> Self {
        Self { inner }
    }
}

impl<D, P> PublicProviderResolver for ClaudeOnlyResolver<D, P>
where
    D: gent_drivers::discovery::ExecutableDiscovery,
    P: gent_drivers::discovery::VersionProbe,
{
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        if provider != "claude" {
            return Err(PublicProviderRunError::CompatibilityDenied);
        }
        self.inner.resolve(provider)
    }
}
