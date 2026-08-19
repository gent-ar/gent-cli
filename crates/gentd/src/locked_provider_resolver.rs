//! Read-only resolution from a durable Gent-owned provider installation lock.
//!
//! This resolver intentionally has no discovery or version-probe dependency. Authority can use
//! only a previously provisioned executable and fails closed when its exact identity changes.

use gent_drivers::lock::recheck;
use gent_ports::{ProvisionedProviderLockReader, PublicProviderResolver, PublicProviderRunError};
use gent_types::RunVersionLock;

/// Resolves an exact ledger-backed installation without PATH or prefix rediscovery.
#[derive(Debug)]
pub(crate) struct LockedProviderResolver<L> {
    installations: L,
}

impl<L> LockedProviderResolver<L> {
    /// Binds the read-only installation provenance source.
    #[must_use]
    pub(crate) const fn new(installations: L) -> Self {
        Self { installations }
    }
}

impl<L: ProvisionedProviderLockReader> PublicProviderResolver for LockedProviderResolver<L> {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        let installation = self
            .installations
            .find_provisioned_provider_installation(provider)
            .map_err(|_| PublicProviderRunError::CompatibilityDenied)?
            .ok_or(PublicProviderRunError::CompatibilityDenied)?;
        let lock = installation.lock.run_lock;
        if lock.provider != provider || recheck(&lock).is_err() {
            return Err(PublicProviderRunError::CompatibilityDenied);
        }
        Ok(lock)
    }
}

#[cfg(test)]
mod tests {
    use gent_drivers::lock::capture;
    use gent_ports::{LedgerError, PublicProviderResolver};
    use gent_types::{
        ProviderInstallProvenance, ProvisionedProviderInstallation, ProvisionedProviderLock,
    };

    use super::LockedProviderResolver;

    #[derive(Clone, Debug)]
    struct Reader(Option<ProvisionedProviderInstallation>);

    impl gent_ports::ProvisionedProviderLockReader for Reader {
        fn find_provisioned_provider_installation(
            &self,
            _: &str,
        ) -> Result<Option<ProvisionedProviderInstallation>, LedgerError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn resolves_only_the_durable_lock_after_rechecking_its_identity() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("codex");
        std::fs::write(&executable, "locked provider binary").unwrap();
        let lock = capture("codex", &executable, "1.0", "compatibility-entry").unwrap();
        let resolver = LockedProviderResolver::new(Reader(Some(installation(lock.clone()))));

        assert_eq!(resolver.resolve("codex").unwrap(), lock);
        assert!(resolver.resolve("claude").is_err());

        std::fs::write(executable, "changed provider binary").unwrap();
        assert!(resolver.resolve("codex").is_err());
    }

    #[test]
    fn missing_durable_installation_never_falls_back_to_discovery() {
        let resolver = LockedProviderResolver::new(Reader(None));
        assert!(resolver.resolve("codex").is_err());
    }

    fn installation(lock: gent_types::RunVersionLock) -> ProvisionedProviderInstallation {
        ProvisionedProviderInstallation {
            lock: ProvisionedProviderLock { run_lock: lock },
            provenance: ProviderInstallProvenance {
                package_name: "@openai/codex".into(),
                package_version: "1.0".into(),
                package_integrity: "sha512-test".into(),
                package_policy_digest_sha256: "a".repeat(64),
                node_runtime_digest_sha256: "b".repeat(64),
                release_artifact_digest_sha256: "c".repeat(64),
                receipt_fingerprint_sha256: "d".repeat(64),
            },
        }
    }
}
