//! Canonical Gent data-directory and local-endpoint resolution.
//!
//! `gent`, `gentd`, and any host that launches the packaged runtime (including a native
//! application embedding it) must resolve through these functions so they always agree on
//! where the local IPC endpoint and durable ledger live. `GENT_DATA_DIR` is the only sanctioned
//! override, intended for development and testing.

use std::env;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

const DATA_DIR_ENV: &str = "GENT_DATA_DIR";
const DATA_DIR_NAME: &str = ".gentd";
const LEGACY_DATA_DIR_NAME: &str = ".gent-cli";
const LOCAL_SOCKET_FILE_NAME: &str = "gentd.sock";

/// Resolves the one canonical Gent data directory.
#[must_use]
pub fn default_data_dir() -> PathBuf {
    resolve_data_dir(env::var_os(DATA_DIR_ENV))
}

fn resolve_data_dir(overridden: Option<OsString>) -> PathBuf {
    match overridden {
        Some(value) if !value.is_empty() => PathBuf::from(value),
        _ => directories::BaseDirs::new().map_or_else(
            || PathBuf::from(DATA_DIR_NAME),
            |directories| directories.home_dir().join(DATA_DIR_NAME),
        ),
    }
}

/// One-time upgrade for an install from before the `.gent-cli` → `.gentd` rename: if the
/// canonical directory does not exist yet but the previous one does, move it into place.
///
/// Only the daemon calls this, and only once, before it first creates or opens the directory
/// (`daemon_bootstrap::run`) — `gentd` is the sole owner of the directory's on-disk lifecycle. It
/// never runs when `GENT_DATA_DIR` is set or `--data-dir` was passed explicitly; an explicit
/// location is never subject to this rename.
///
/// # Errors
/// Returns an error if the previous directory exists, the canonical one does not, and the rename
/// itself fails (for example a permissions error, or a legacy directory on a different filesystem
/// than the home directory).
pub fn migrate_legacy_default_data_dir() -> std::io::Result<()> {
    if env::var_os(DATA_DIR_ENV).is_some() {
        return Ok(());
    }
    let Some(directories) = directories::BaseDirs::new() else {
        return Ok(());
    };
    migrate_legacy_data_dir_under(directories.home_dir())
}

fn migrate_legacy_data_dir_under(home: &Path) -> std::io::Result<()> {
    let current = home.join(DATA_DIR_NAME);
    let legacy = home.join(LEGACY_DATA_DIR_NAME);
    if current.exists() || !legacy.exists() {
        return Ok(());
    }
    std::fs::rename(legacy, current)
}

/// The Unix-domain socket path for a data directory's local IPC endpoint.
#[must_use]
pub fn local_socket_path(data_dir: &Path) -> PathBuf {
    data_dir.join(LOCAL_SOCKET_FILE_NAME)
}

/// The Windows named-pipe name for a data directory's local IPC endpoint.
///
/// Every client that speaks to a Windows `gentd` (the CLI, the daemon itself, and a native
/// host's byte transport) must derive this name from the same hash so they open the same pipe.
#[must_use]
pub fn windows_pipe_name(data_dir: &Path) -> String {
    format!(r"\\.\pipe\gentd-{:016x}", windows_endpoint_hash(data_dir))
}

#[must_use]
fn windows_endpoint_hash(data_dir: &Path) -> u64 {
    data_dir
        .to_string_lossy()
        .bytes()
        .fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
            (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
        })
}

/// Finds a same-directory-tree sibling executable, walking up from the current process.
///
/// This is how a client (such as `gent`) locates the `gentd` binary it should launch without
/// depending on `PATH`: packaged installs place every Gent binary under one runtime root.
/// Falls back to a bare `name`, which resolves through `PATH` when spawned.
#[must_use]
pub fn resolve_sibling_binary(name: &str) -> PathBuf {
    let Some(executable) = env::current_exe().ok() else {
        return PathBuf::from(name);
    };
    let mut directory = executable.parent();
    while let Some(path) = directory {
        let candidate = path.join(name);
        if candidate.is_file() {
            return candidate;
        }
        directory = path.parent();
    }
    PathBuf::from(name)
}

#[cfg(test)]
mod tests {
    use super::{
        local_socket_path, migrate_legacy_data_dir_under, resolve_data_dir, windows_pipe_name,
    };
    use std::ffi::OsString;
    use std::path::Path;

    #[test]
    fn resolve_data_dir_honors_a_non_empty_override() {
        assert_eq!(
            resolve_data_dir(Some(OsString::from("/tmp/gent-paths-test-override"))),
            Path::new("/tmp/gent-paths-test-override")
        );
    }

    #[test]
    fn resolve_data_dir_falls_back_to_home_dot_gentd_when_unset_or_empty() {
        assert!(resolve_data_dir(None).ends_with(".gentd"));
        assert!(resolve_data_dir(Some(OsString::new())).ends_with(".gentd"));
    }

    #[test]
    fn migration_moves_a_legacy_directory_into_place() {
        let home = tempfile::tempdir().unwrap();
        let legacy = home.path().join(".gent-cli");
        std::fs::create_dir(&legacy).unwrap();
        std::fs::write(legacy.join("ledger.sqlite"), b"data").unwrap();

        migrate_legacy_data_dir_under(home.path()).unwrap();

        let current = home.path().join(".gentd");
        assert!(current.is_dir());
        assert!(!legacy.exists());
        assert_eq!(std::fs::read(current.join("ledger.sqlite")).unwrap(), b"data");
    }

    #[test]
    fn migration_is_a_no_op_when_the_canonical_directory_already_exists() {
        let home = tempfile::tempdir().unwrap();
        std::fs::create_dir(home.path().join(".gent-cli")).unwrap();
        std::fs::create_dir(home.path().join(".gentd")).unwrap();
        std::fs::write(home.path().join(".gentd").join("marker"), b"keep").unwrap();

        migrate_legacy_data_dir_under(home.path()).unwrap();

        assert!(home.path().join(".gent-cli").exists());
        assert_eq!(
            std::fs::read(home.path().join(".gentd").join("marker")).unwrap(),
            b"keep"
        );
    }

    #[test]
    fn migration_is_a_no_op_when_there_is_no_legacy_directory() {
        let home = tempfile::tempdir().unwrap();
        migrate_legacy_data_dir_under(home.path()).unwrap();
        assert!(!home.path().join(".gentd").exists());
    }

    #[test]
    fn local_socket_path_is_gentd_sock_under_the_data_dir() {
        assert_eq!(
            local_socket_path(Path::new("/data")),
            Path::new("/data/gentd.sock")
        );
    }

    #[test]
    fn windows_pipe_name_is_deterministic_for_the_same_data_dir() {
        let first = windows_pipe_name(Path::new(r"C:\gent\data"));
        let second = windows_pipe_name(Path::new(r"C:\gent\data"));
        assert_eq!(first, second);
        assert_ne!(first, windows_pipe_name(Path::new(r"C:\gent\other")));
        assert!(first.starts_with(r"\\.\pipe\gentd-"));
    }
}
