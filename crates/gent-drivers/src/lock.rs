//! Immutable provider executable identity checks; this module never spawns a provider.

use std::fs;
use std::path::Path;
use std::time::UNIX_EPOCH;

use gent_types::RunVersionLock;
use sha2::{Digest, Sha256};

#[derive(Debug, thiserror::Error)]
pub enum LockError {
    #[error("provider executable changed before spawn")]
    ProviderChanged,
    #[error("cannot inspect provider executable: {0}")]
    Io(#[from] std::io::Error),
}

/// Captures canonical identity and digest using an already-discovered version string.
///
/// # Errors
/// Returns an error when the executable cannot be resolved or read.
pub fn capture(
    provider: &str,
    executable: &Path,
    version: &str,
    entry: &str,
) -> Result<RunVersionLock, LockError> {
    let canonical_path = fs::canonicalize(executable)?;
    let metadata = fs::metadata(&canonical_path)?;
    let bytes = fs::read(&canonical_path)?;
    Ok(RunVersionLock {
        provider: provider.into(),
        canonical_path: canonical_path.display().to_string(),
        file_identity: file_identity(&metadata)?,
        digest_sha256: hex::encode(Sha256::digest(bytes)),
        version: version.into(),
        compatibility_entry: entry.into(),
    })
}

/// Rechecks an immutable run lock immediately before a spawn or resume.
///
/// # Errors
/// Returns [`LockError::ProviderChanged`] instead of silently substituting a binary.
pub fn recheck(lock: &RunVersionLock) -> Result<(), LockError> {
    let path = Path::new(&lock.canonical_path);
    let metadata = fs::metadata(path)?;
    let current_identity = file_identity(&metadata)?;
    (current_identity == lock.file_identity)
        .then_some(())
        .ok_or(LockError::ProviderChanged)
}

/// Re-discovers the immutable executable identity for a trusted launch preflight.
///
/// This returns no authorization decision and never launches a process. A sandbox-preflight edge
/// compares the result against its saved lock before it can produce an attestation.
///
/// # Errors
/// Returns an error when the executable cannot be resolved or read.
pub fn rechecked_identity(lock: &RunVersionLock) -> Result<RunVersionLock, LockError> {
    capture(
        &lock.provider,
        Path::new(&lock.canonical_path),
        &lock.version,
        &lock.compatibility_entry,
    )
}

fn file_identity(metadata: &fs::Metadata) -> Result<String, LockError> {
    Ok(format!(
        "{}:{}",
        metadata.len(),
        metadata
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_or(0, |duration| duration.as_nanos())
    ))
}

#[cfg(test)]
mod tests {
    use super::{LockError, capture, recheck, rechecked_identity};
    use std::fs;

    #[test]
    fn changed_binary_is_not_silently_accepted() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("provider");
        fs::write(&executable, "first").unwrap();
        let lock = capture("claude", &executable, "1", "entry").unwrap();
        fs::write(&executable, "second").unwrap();
        assert!(matches!(recheck(&lock), Err(LockError::ProviderChanged)));
    }

    #[test]
    fn rechecked_identity_retains_the_exact_saved_lock_when_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("provider");
        fs::write(&executable, "first").unwrap();
        let lock = capture("claude", &executable, "1", "entry").unwrap();
        assert_eq!(rechecked_identity(&lock).unwrap(), lock);
    }

    #[test]
    fn pre_spawn_recheck_does_not_read_the_saved_binary_when_metadata_is_unchanged() {
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("provider");
        fs::write(&executable, "first").unwrap();
        let lock = capture("claude", &executable, "1", "entry").unwrap();
        recheck(&lock).unwrap();
    }
}
