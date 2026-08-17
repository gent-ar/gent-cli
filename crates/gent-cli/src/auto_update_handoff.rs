//! External, opt-in runtime update scheduler handoff.

use std::path::PathBuf;
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
    let release = executable.parent().ok_or(AutoUpdateError::NotInstalled)?;
    let releases = release.parent().ok_or(AutoUpdateError::NotInstalled)?;
    let root = releases.parent().ok_or(AutoUpdateError::NotInstalled)?;
    (releases.file_name().is_some_and(|name| name == "releases")
        && root.join("current").is_symlink())
    .then_some(root.to_path_buf())
    .ok_or(AutoUpdateError::NotInstalled)
}

#[cfg(test)]
mod tests {
    use super::runtime_root;

    #[test]
    fn development_binary_is_not_mistaken_for_an_installed_runtime() {
        if std::env::current_exe().is_ok_and(|path| path.to_string_lossy().contains("target")) {
            assert!(runtime_root().is_err());
        }
    }
}
