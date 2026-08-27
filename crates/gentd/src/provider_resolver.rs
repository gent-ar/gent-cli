use gent_ports::{PublicProviderResolver, PublicProviderRunError};
use gent_types::RunVersionLock;

#[derive(Debug)]
pub(crate) struct CodexOnlyResolver<R> {
    inner: R,
}

impl<R> CodexOnlyResolver<R> {
    #[must_use]
    pub(crate) const fn new(inner: R) -> Self {
        Self { inner }
    }
}

impl<R: PublicProviderResolver> PublicProviderResolver for CodexOnlyResolver<R> {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        if provider != "codex" {
            return Err(PublicProviderRunError::CompatibilityDenied);
        }
        self.inner.resolve(provider)
    }
}
