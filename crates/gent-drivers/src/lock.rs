//! Immutable provider executable identity checks; this module never spawns a provider.

use std::fs;
use std::io::Read;
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
    capture_observed(provider, executable, version, entry, |_| {})
}

fn capture_observed(
    provider: &str,
    executable: &Path,
    version: &str,
    entry: &str,
    mut observed: impl FnMut(u64),
) -> Result<RunVersionLock, LockError> {
    let canonical_path = fs::canonicalize(executable)?;
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    let mut file = options.open(&canonical_path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(LockError::ProviderChanged);
    }
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 65536];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        total = total
            .checked_add(count as u64)
            .ok_or(LockError::ProviderChanged)?;
        observed(total);
    }
    let after = file.metadata()?;
    let path_after = fs::metadata(&canonical_path)?;
    if total != metadata.len()
        || !same_snapshot(&metadata, &after)?
        || !same_snapshot(&metadata, &path_after)?
    {
        return Err(LockError::ProviderChanged);
    }
    Ok(RunVersionLock {
        provider: provider.into(),
        canonical_path: canonical_path.display().to_string(),
        file_identity: file_identity(&metadata)?,
        digest_sha256: hex::encode(digest.finalize()),
        version: version.into(),
        compatibility_entry: entry.into(),
    })
}

fn same_snapshot(before: &fs::Metadata, after: &fs::Metadata) -> Result<bool, LockError> {
    if file_identity(before)? != file_identity(after)? {
        return Ok(false);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        return Ok((
            before.dev(),
            before.ino(),
            before.ctime(),
            before.ctime_nsec(),
        ) == (after.dev(), after.ino(), after.ctime(), after.ctime_nsec()));
    }
    #[cfg(not(unix))]
    Ok(true)
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
    fn streaming_capture_preserves_digest_and_public_identity_format() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider");
        fs::write(&path, b"abc").unwrap();
        let lock = capture("claude", &path, "version", "entry").unwrap();
        assert_eq!(
            lock.digest_sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            lock.file_identity,
            super::file_identity(&fs::metadata(&path).unwrap()).unwrap()
        );
        assert_eq!(lock.provider, "claude");
        assert_eq!(lock.version, "version");
        assert_eq!(lock.compatibility_entry, "entry");
    }

    #[cfg(unix)]
    #[test]
    fn edit_during_hash_with_restored_mtime_is_rejected() {
        use std::io::{Seek, SeekFrom, Write};
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider");
        fs::write(&path, vec![b'a'; 3 * 65536]).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        let mut changed = false;
        let result = super::capture_observed("claude", &path, "version", "entry", |_| {
            if changed {
                return;
            }
            changed = true;
            let mut file = fs::OpenOptions::new().write(true).open(&path).unwrap();
            file.seek(SeekFrom::End(-1)).unwrap();
            file.write_all(b"b").unwrap();
            file.set_times(fs::FileTimes::new().set_modified(modified))
                .unwrap();
        });
        assert!(changed);
        assert!(matches!(result, Err(LockError::ProviderChanged)));
    }

    #[cfg(unix)]
    #[test]
    fn equal_content_and_mtime_replacement_during_hash_is_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("provider");
        let replacement = directory.path().join("replacement");
        let bytes = vec![b'a'; 3 * 65536];
        fs::write(&path, &bytes).unwrap();
        fs::write(&replacement, &bytes).unwrap();
        let modified = fs::metadata(&path).unwrap().modified().unwrap();
        fs::OpenOptions::new()
            .write(true)
            .open(&replacement)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(modified))
            .unwrap();
        let mut changed = false;
        let result = super::capture_observed("claude", &path, "version", "entry", |_| {
            if changed {
                return;
            }
            changed = true;
            fs::rename(&replacement, &path).unwrap();
        });
        assert!(changed);
        assert!(matches!(result, Err(LockError::ProviderChanged)));
    }

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
