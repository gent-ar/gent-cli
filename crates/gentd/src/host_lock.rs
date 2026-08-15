//! Single-host process ownership; retained for the whole daemon lifetime.

use std::fs::{File, OpenOptions};
use std::path::Path;

use fs2::FileExt;

#[derive(Debug)]
pub(super) struct HostLock(File);

/// Acquires exclusive daemon ownership for `data_dir`.
///
/// # Errors
/// Returns an error if another `gentd` process owns the same data directory.
pub(super) fn acquire(data_dir: &Path) -> Result<HostLock, std::io::Error> {
    let file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(data_dir.join("gentd.lock"))?;
    file.try_lock_exclusive().map_err(|error| {
        std::io::Error::new(
            error.kind(),
            format!(
                "another gentd instance owns {}: {error}",
                data_dir.display()
            ),
        )
    })?;
    Ok(HostLock(file))
}

impl Drop for HostLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::acquire;

    #[test]
    fn only_one_host_can_own_a_data_directory() {
        let directory = tempfile::tempdir().unwrap();
        let first = acquire(directory.path()).unwrap();
        assert!(acquire(directory.path()).is_err());
        drop(first);
        assert!(acquire(directory.path()).is_ok());
    }
}
