//! External runtime update scheduler handoff.

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
    let helper = runtime_root.join(helper_name());
    let metadata = std::fs::symlink_metadata(&helper).map_err(|_| AutoUpdateError::UnsafeHelper)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(AutoUpdateError::UnsafeHelper);
    }
    let mut command = Command::new(helper_runner());
    #[cfg(windows)]
    command
        .arg("-NoProfile")
        .arg("-NonInteractive")
        .arg("-ExecutionPolicy")
        .arg("Bypass")
        .arg("-File")
        .arg(helper)
        .arg(action.name())
        .arg("-RuntimeRoot")
        .arg(runtime_root)
        .arg("-DataDir")
        .arg(data_dir);
    #[cfg(not(windows))]
    command
        .arg(helper)
        .arg(action.name())
        .arg("--runtime-root")
        .arg(runtime_root)
        .arg("--data-dir")
        .arg(data_dir);
    if let Some(interval) = action.interval_seconds() {
        #[cfg(windows)]
        command.arg("-IntervalSeconds").arg(interval.to_string());
        #[cfg(not(windows))]
        command.arg("--interval-seconds").arg(interval.to_string());
    }
    if action.force() {
        #[cfg(windows)]
        command.arg("-Force");
        #[cfg(not(windows))]
        command.arg("--force");
    }
    if let Some(directory) = std::env::var_os("GENT_AUTO_UPDATE_SCHEDULER_DIR") {
        #[cfg(windows)]
        command.arg("-SchedulerDir").arg(directory);
        #[cfg(not(windows))]
        command.arg("--scheduler-dir").arg(directory);
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
    #[cfg(windows)]
    {
        if executable.file_name().is_none_or(|name| name != "gent.exe") {
            return None;
        }
        let parent = executable.parent()?;
        // `windows_launcher.rs` always spawns the release copy as a child
        // process rather than replacing itself (Windows has no exec-in-place),
        // so an ordinary `gent update ...` invocation reports its own
        // `current_exe()` as `root/releases/<release>/gent.exe`, not
        // `root/bin/gent.exe`. Recognize both shapes.
        let root = if parent.file_name().is_some_and(|name| name == "bin") {
            parent.parent()?
        } else if parent.parent()?.file_name().is_some_and(|name| name == "releases") {
            parent.parent()?.parent()?
        } else {
            return None;
        };
        let release = std::fs::read_to_string(root.join("current.json")).ok()?;
        let metadata = serde_json::from_str::<serde_json::Value>(&release).ok()?;
        let release = metadata.get("release")?.as_str()?;
        let expected = root.join("releases").join(release).join("gent.exe");
        // The running executable must be exactly one of the two recognized,
        // currently-selected copies — never a stray or stale `gent.exe`
        // elsewhere that happens to share a directory shape.
        let is_current_copy = executable == root.join("bin").join("gent.exe") || executable == expected;
        (is_current_copy && expected.is_file() && root.join("releases").is_dir()).then(|| root.to_path_buf())
    }
    #[cfg(not(windows))]
    {
        let executable = executable.canonicalize().ok()?;
        let release = executable.parent()?;
        let releases = release.parent()?;
        let root = releases.parent()?;
        let active_release = root.join("current").canonicalize().ok()?;
        (releases.file_name().is_some_and(|name| name == "releases") && active_release == release)
            .then(|| root.to_path_buf())
    }
}

#[cfg(windows)]
const fn helper_name() -> &'static str {
    "gent-auto-update.ps1"
}
#[cfg(not(windows))]
const fn helper_name() -> &'static str {
    "gent-auto-update.py"
}
#[cfg(windows)]
const fn helper_runner() -> &'static str {
    "powershell.exe"
}
#[cfg(not(windows))]
const fn helper_runner() -> &'static str {
    "python3"
}

#[cfg(test)]
mod tests {
    use super::runtime_root;
    #[cfg(unix)]
    use super::runtime_root_from_executable;

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

    #[cfg(windows)]
    #[test]
    fn windows_bin_launcher_path_resolves_to_the_active_managed_runtime() {
        use super::runtime_root_from_executable;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("runtime");
        let release = root.join("releases").join("v1.2.3-x86_64-pc-windows-msvc");
        std::fs::create_dir_all(&release).unwrap();
        std::fs::write(release.join("gent.exe"), b"fixture").unwrap();
        std::fs::create_dir_all(root.join("bin")).unwrap();
        std::fs::write(
            root.join("current.json"),
            r#"{"release":"v1.2.3-x86_64-pc-windows-msvc"}"#,
        )
        .unwrap();
        assert_eq!(
            runtime_root_from_executable(&root.join("bin").join("gent.exe")),
            Some(root)
        );
    }

    /// The regression case: `windows_launcher.rs` spawns the release copy as
    /// a child rather than replacing itself in place, so this is the path an
    /// ordinary `gent update ...` invocation actually runs from.
    #[cfg(windows)]
    #[test]
    fn windows_release_child_path_resolves_to_the_active_managed_runtime() {
        use super::runtime_root_from_executable;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("runtime");
        let release = root.join("releases").join("v1.2.3-x86_64-pc-windows-msvc");
        std::fs::create_dir_all(&release).unwrap();
        std::fs::write(release.join("gent.exe"), b"fixture").unwrap();
        std::fs::write(
            root.join("current.json"),
            r#"{"release":"v1.2.3-x86_64-pc-windows-msvc"}"#,
        )
        .unwrap();
        assert_eq!(
            runtime_root_from_executable(&release.join("gent.exe")),
            Some(root)
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_stray_gent_exe_next_to_an_unrelated_directory_is_not_installed() {
        use super::runtime_root_from_executable;

        let directory = tempfile::tempdir().unwrap();
        let stray = directory.path().join("Downloads").join("gent.exe");
        std::fs::create_dir_all(stray.parent().unwrap()).unwrap();
        std::fs::write(&stray, b"fixture").unwrap();
        assert_eq!(runtime_root_from_executable(&stray), None);
    }

    #[cfg(windows)]
    #[test]
    fn windows_release_child_from_a_release_current_json_no_longer_selects_is_not_installed() {
        use super::runtime_root_from_executable;

        let directory = tempfile::tempdir().unwrap();
        let root = directory.path().join("runtime");
        let stale = root.join("releases").join("v1.2.2-x86_64-pc-windows-msvc");
        std::fs::create_dir_all(&stale).unwrap();
        std::fs::write(stale.join("gent.exe"), b"fixture").unwrap();
        let current = root.join("releases").join("v1.2.3-x86_64-pc-windows-msvc");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("gent.exe"), b"fixture").unwrap();
        std::fs::write(
            root.join("current.json"),
            r#"{"release":"v1.2.3-x86_64-pc-windows-msvc"}"#,
        )
        .unwrap();
        assert_eq!(runtime_root_from_executable(&stale.join("gent.exe")), None);
    }
}
