//! Single-host process ownership; retained for the whole daemon lifetime.
//!
//! The lock file also carries a best-effort owner record (pid and build version) so a
//! conflicting launch — for example a native app spawning `gentd` into a data directory
//! `gent` already owns — can report who already owns it instead of a bare OS lock error.

use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;

use fs2::FileExt;

#[derive(Debug)]
pub(super) struct HostLock(File);

/// The owner recorded in a data directory's lock file, read on a failed acquire.
#[derive(Debug, Eq, PartialEq)]
pub(super) struct LockOwner {
    pub(super) pid: u32,
    pub(super) version: String,
}

/// Acquires exclusive daemon ownership for `data_dir`.
///
/// # Errors
/// Returns an error if another `gentd` process owns the same data directory. The error message
/// names that owner's process id and build version when the holder recorded them.
pub(super) fn acquire(data_dir: &Path) -> Result<HostLock, std::io::Error> {
    let mut file = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(data_dir.join("gentd.lock"))?;
    if let Err(error) = file.try_lock_exclusive() {
        let owner = read_owner(&mut file);
        return Err(std::io::Error::new(
            error.kind(),
            describe_conflict(data_dir, &error, owner),
        ));
    }
    write_owner(&mut file, current_owner())?;
    Ok(HostLock(file))
}

fn current_owner() -> LockOwner {
    LockOwner {
        pid: std::process::id(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    }
}

fn describe_conflict(data_dir: &Path, error: &std::io::Error, owner: Option<LockOwner>) -> String {
    let directory = data_dir.display();
    owner.map_or_else(
        || format!("another gentd instance owns {directory}: {error}"),
        |owner| {
            format!(
                "gentd pid {} (version {}) already owns {directory}: {error}",
                owner.pid, owner.version
            )
        },
    )
}

fn write_owner(file: &mut File, owner: LockOwner) -> Result<(), std::io::Error> {
    file.set_len(0)?;
    file.seek(SeekFrom::Start(0))?;
    writeln!(file, "{}\n{}", owner.pid, owner.version)?;
    file.flush()
}

fn read_owner(file: &mut File) -> Option<LockOwner> {
    let mut contents = String::new();
    file.seek(SeekFrom::Start(0)).ok()?;
    file.read_to_string(&mut contents).ok()?;
    let mut lines = contents.lines();
    let pid: u32 = lines.next()?.trim().parse().ok()?;
    let version = lines.next()?.trim().to_string();
    (!version.is_empty()).then_some(LockOwner { pid, version })
}

impl Drop for HostLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::{acquire, current_owner, describe_conflict, read_owner, write_owner};

    #[test]
    fn only_one_host_can_own_a_data_directory() {
        let directory = tempfile::tempdir().unwrap();
        let first = acquire(directory.path()).unwrap();
        assert!(acquire(directory.path()).is_err());
        drop(first);
        assert!(acquire(directory.path()).is_ok());
    }

    #[test]
    fn a_conflicting_acquire_names_the_current_process_as_owner() {
        let directory = tempfile::tempdir().unwrap();
        let _first = acquire(directory.path()).unwrap();
        let error = acquire(directory.path()).unwrap_err();
        let message = error.to_string();
        assert!(message.contains(&std::process::id().to_string()));
        assert!(message.contains(env!("CARGO_PKG_VERSION")));
    }

    #[test]
    fn owner_round_trips_through_the_lock_file() {
        let directory = tempfile::tempdir().unwrap();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(directory.path().join("owner.txt"))
            .unwrap();
        let owner = current_owner();
        write_owner(&mut file, current_owner()).unwrap();
        assert_eq!(read_owner(&mut file), Some(owner));
    }

    #[test]
    fn a_lock_file_from_an_older_gentd_without_owner_metadata_still_reports_a_conflict() {
        let directory = tempfile::tempdir().unwrap();
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .open(directory.path().join("empty.txt"))
            .unwrap();
        assert_eq!(read_owner(&mut file), None);
        let error = std::io::Error::other("would block");
        assert!(describe_conflict(directory.path(), &error, None).contains("another gentd"));
    }
}
