use std::{
    ffi::OsString,
    path::{Path, PathBuf},
};

pub(super) fn enable_updates(root: &Path, data_dir: &Path) -> Result<(), String> {
    let status = gent_types::hidden_command(active_cli(root))
        .args(enable_update_arguments(data_dir))
        .status()
        .map_err(super::display)?;
    if status.success() {
        Ok(())
    } else {
        Err("Gent automatic updates could not be enabled".into())
    }
}

pub(super) fn enable_update_arguments(data_dir: &Path) -> Vec<OsString> {
    vec![
        "--data-dir".into(),
        data_dir.as_os_str().to_owned(),
        "update".into(),
        "auto".into(),
        "enable".into(),
    ]
}

pub(super) fn active_cli(root: &Path) -> PathBuf {
    root.join(if cfg!(windows) {
        "bin/gent.exe"
    } else {
        "current/gent"
    })
}

pub(super) const fn auto_update_helper_name() -> &'static str {
    if cfg!(windows) {
        "gent-auto-update.ps1"
    } else {
        "gent-auto-update.py"
    }
}

#[cfg(test)]
pub(super) fn default_root() -> PathBuf {
    #[cfg(windows)]
    {
        PathBuf::from(std::env::var_os("LOCALAPPDATA").expect("LOCALAPPDATA is required"))
            .join("Gent")
    }
    #[cfg(not(windows))]
    {
        PathBuf::from(std::env::var_os("HOME").expect("HOME is required")).join(".local/lib/gent")
    }
}
