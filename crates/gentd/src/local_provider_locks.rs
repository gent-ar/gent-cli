//! Explicit local executable locks for a future standalone authority profile.
//!
//! This is deliberately not discovery: callers must provide canonical candidate paths. It makes
//! the exact resolver seam available without falling back to `PATH`, a provider prefix, or the
//! signed ordinary-authority release path.

use std::{collections::BTreeMap, path::Path};

use gent_drivers::{
    PublicProvider,
    lock::{LockError, capture, recheck},
};
use gent_ports::{PublicProviderResolver, PublicProviderRunError};
use gent_types::RunVersionLock;

/// Resolver input for one explicitly selected local public executable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct LocalProviderLocks {
    locks: BTreeMap<String, RunVersionLock>,
}

/// Controlled errors while forming a standalone local executable resolver.
#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub(crate) enum LocalProviderLockError {
    #[error("local provider path is unavailable")]
    PathUnavailable,
    #[error("local provider path is not a file")]
    NotAFile,
    #[error("local provider was selected more than once")]
    DuplicateProvider,
    #[error("local provider identity cannot be captured")]
    Capture,
}

impl LocalProviderLocks {
    /// Captures exact file identities from explicitly selected paths, without starting a process.
    ///
    /// The placeholder version is intentionally not a compatibility claim. A future standalone
    /// authority must pair these locks with its own local compatibility authorizer before launch.
    pub(crate) fn capture(
        providers: impl IntoIterator<Item = (PublicProvider, std::path::PathBuf)>,
    ) -> Result<Self, LocalProviderLockError> {
        let mut locks = BTreeMap::new();
        for (provider, path) in providers {
            let lock = capture_local(provider, &path)?;
            if locks
                .insert(provider.executable_name().into(), lock)
                .is_some()
            {
                return Err(LocalProviderLockError::DuplicateProvider);
            }
        }
        Ok(Self { locks })
    }
}

impl PublicProviderResolver for LocalProviderLocks {
    fn resolve(&self, provider: &str) -> Result<RunVersionLock, PublicProviderRunError> {
        let lock = self
            .locks
            .get(provider)
            .ok_or(PublicProviderRunError::CompatibilityDenied)?;
        recheck(lock).map_err(lock_error)?;
        Ok(lock.clone())
    }
}

fn capture_local(
    provider: PublicProvider,
    path: &Path,
) -> Result<RunVersionLock, LocalProviderLockError> {
    let path = path
        .canonicalize()
        .map_err(|_| LocalProviderLockError::PathUnavailable)?;
    if !path.is_file() {
        return Err(LocalProviderLockError::NotAFile);
    }
    capture(
        provider.executable_name(),
        &path,
        "local-unprobed",
        "standalone-local-v1",
    )
    .map_err(|_| LocalProviderLockError::Capture)
}

fn lock_error(error: LockError) -> PublicProviderRunError {
    match error {
        LockError::ProviderChanged => PublicProviderRunError::ProviderChanged,
        LockError::Io(_) => PublicProviderRunError::CompatibilityDenied,
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use gent_drivers::PublicProvider;
    use gent_ports::PublicProviderResolver;

    use super::{LocalProviderLockError, LocalProviderLocks};

    #[test]
    fn captures_explicit_paths_without_path_discovery_or_process_start() {
        let directory = tempfile::tempdir().unwrap();
        let claude = directory.path().join("chosen-claude");
        fs::write(&claude, "claude executable").unwrap();
        let locks = LocalProviderLocks::capture([(PublicProvider::Claude, claude)]).unwrap();

        let lock = locks.resolve("claude").unwrap();
        assert_eq!(lock.provider, "claude");
        assert_eq!(lock.version, "local-unprobed");
        assert_eq!(lock.compatibility_entry, "standalone-local-v1");
        assert!(locks.resolve("codex").is_err());
    }

    #[test]
    fn refuses_mutation_and_duplicate_provider_selection() {
        let directory = tempfile::tempdir().unwrap();
        let claude = directory.path().join("claude");
        fs::write(&claude, "before").unwrap();
        let locks =
            LocalProviderLocks::capture([(PublicProvider::Claude, claude.clone())]).unwrap();
        fs::write(&claude, "after").unwrap();
        assert!(locks.resolve("claude").is_err());

        assert_eq!(
            LocalProviderLocks::capture([
                (PublicProvider::Claude, claude.clone()),
                (PublicProvider::Claude, claude),
            ]),
            Err(LocalProviderLockError::DuplicateProvider)
        );
    }
}
