//! External, opt-in runtime update scheduler handoff.

use std::path::{Path, PathBuf};
use std::process::Command;

use thiserror::Error;

use crate::update_check::AutoUpdateAction;

#[derive(Debug, Error)]
pub(crate) enum AutoUpdateError {
    #[error("Gent automatic updates require an installed paired runtime")]
    NotInstalled,
    #[error("the installed automatic-update helper is missing or unsafe")]
    UnsafeHelper,
    #[error("could not start the external automatic-update helper: {0}")]
    Start(#[from] std::io::Error),
    #[error("the external automatic-update helper rejected the request")]
    Rejected,
}

/// Invokes the signed helper next to the active runtime; never contacts a release source itself.
pub(crate) fn invoke(action: &AutoUpdateAction, data_dir: PathBuf) -> Result<(), AutoUpdateError> {
    let runtime_root = runtime_root()?;
    let helper = runtime_root.join("gent-auto-update.py");
    let metadata = std::fs::symlink_metadata(&helper).map_err(|_| AutoUpdateError::UnsafeHelper)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AutoUpdateError::UnsafeHelper);
    }
    let mut command = Command::new("python3");
    command
        .arg(helper)
        .arg(action.name())
        .arg("--runtime-root")
        .arg(runtime_root)
        .arg("--data-dir")
        .arg(data_dir);
    if let Some(interval) = action.interval_seconds() {
        command.arg("--interval-seconds").arg(interval.to_string());
    }
    if action.force() {
        command.arg("--force");
    }
    command
        .status()?
        .success()
        .then_some(())
        .ok_or(AutoUpdateError::Rejected)
}

fn runtime_root() -> Result<PathBuf, AutoUpdateError> {
    let executable = std::env::current_exe().map_err(|_| AutoUpdateError::NotInstalled)?;
    runtime_root_from_executable(&executable).ok_or(AutoUpdateError::NotInstalled)
}

fn runtime_root_from_executable(executable: &Path) -> Option<PathBuf> {
    let executable = executable.canonicalize().ok()?;
    let release = executable.parent()?;
    let releases = release.parent()?;
    let root = releases.parent()?;
    let active_release = root.join("current").canonicalize().ok()?;
    (releases.file_name().is_some_and(|name| name == "releases") && active_release == release)
        .then(|| root.to_path_buf())
}

#[cfg(test)]
mod tests {
    use super::{runtime_root, runtime_root_from_executable};

    #[test]
    fn development_binary_is_not_mistaken_for_an_installed_runtime() {
        if std::env::current_exe().is_ok_and(|path| path.to_string_lossy().contains("target")) {
            assert!(runtime_root().is_err());
        }
    }

    #[cfg(unix)]
    #[test]
    fn launcher_path_resolves_to_the_active_managed_runtime() {
        use std::os::unix::fs::symlink;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("runtime");
        let release = root.join("releases/v1.2.3-aarch64-apple-darwin");
        std::fs::create_dir_all(&release).unwrap();
        std::fs::write(release.join("gent"), b"fixture").unwrap();
        symlink("releases/v1.2.3-aarch64-apple-darwin", root.join("current")).unwrap();
        assert_eq!(
            runtime_root_from_executable(&root.join("current/gent")),
            Some(root.canonicalize().unwrap())
        );
    }
}
