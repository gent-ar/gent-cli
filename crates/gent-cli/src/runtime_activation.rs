use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use clap::Subcommand;
use serde::Deserialize;

#[path = "runtime_activation_files.rs"]
mod files;

#[derive(Debug, Subcommand)]
pub(crate) enum RuntimeCommand {
    Activate {
        #[arg(long)]
        bootstrap_dir: PathBuf,
        #[arg(long)]
        runtime_root: Option<PathBuf>,
    },
}

#[derive(Deserialize)]
struct Bootstrap {
    version: String,
    target: String,
}

pub(crate) fn activate(
    bootstrap: PathBuf,
    root: Option<PathBuf>,
    data_dir: PathBuf,
) -> Result<PathBuf, String> {
    let root = root.unwrap_or_else(default_root);
    activate_at(&bootstrap, &root, &data_dir, enable_updates)
}

fn activate_at(
    bootstrap: &Path,
    root: &Path,
    data_dir: &Path,
    enable_scheduler: fn(&Path, &Path) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let descriptor: Bootstrap = serde_json::from_str(
        &fs::read_to_string(bootstrap.join("bootstrap.json")).map_err(display)?,
    )
    .map_err(display)?;
    let target = expected_target()?;
    if descriptor.target != target || !valid_version(&descriptor.version) {
        return Err("Gent bootstrap metadata does not match this platform".into());
    }
    verify_bootstrap(bootstrap)?;
    let release = format!("{}-{}", descriptor.version, descriptor.target);
    let releases = root.join("releases");
    let destination = releases.join(&release);
    fs::create_dir_all(&releases).map_err(display)?;
    if destination.exists() {
        if !files::same_tree(bootstrap, &destination)? {
            return Err("Gent managed release differs from the verified bootstrap".into());
        }
    } else {
        let stage = root.join(format!(".stage-{}-{}", release, std::process::id()));
        files::remove_path(&stage)?;
        files::copy_tree(bootstrap, &stage)?;
        fs::rename(&stage, &destination).map_err(display)?;
    }
    let current = selected_release(root)?;
    if current
        .as_ref()
        .is_none_or(|value| version_of(&release) > version_of(value))
    {
        select_release(root, &release)?;
    }
    enable_scheduler(root, data_dir)?;
    Ok(active_daemon(root))
}

fn verify_bootstrap(root: &Path) -> Result<(), String> {
    for name in required_files() {
        let metadata = fs::symlink_metadata(root.join(name)).map_err(display)?;
        if !metadata.file_type().is_file() || metadata.file_type().is_symlink() {
            return Err("Gent bootstrap runtime is incomplete".into());
        }
    }
    Ok(())
}

fn selected_release(root: &Path) -> Result<Option<String>, String> {
    #[cfg(windows)]
    {
        let pointer = root.join("current.json");
        if !pointer.exists() {
            return Ok(None);
        }
        let value: serde_json::Value =
            serde_json::from_str(&fs::read_to_string(pointer).map_err(display)?)
                .map_err(display)?;
        return value
            .get("release")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .ok_or_else(|| "Gent current runtime pointer is invalid".into())
            .map(Some);
    }
    #[cfg(not(windows))]
    {
        let pointer = root.join("current");
        if !pointer.exists() {
            return Ok(None);
        }
        let target = fs::read_link(pointer).map_err(display)?;
        return target
            .file_name()
            .and_then(|value| value.to_str())
            .map(str::to_owned)
            .ok_or_else(|| "Gent current runtime pointer is invalid".into())
            .map(Some);
    }
}

fn select_release(root: &Path, release: &str) -> Result<(), String> {
    #[cfg(windows)]
    {
        let bin = root.join("bin");
        fs::create_dir_all(&bin).map_err(display)?;
        let launcher = root
            .join("releases")
            .join(release)
            .join("gent-launcher.exe");
        for name in ["gent.exe", "gentd.exe"] {
            replace_file(&launcher, &bin.join(name))?;
        }
        replace_bytes(
            serde_json::to_vec(&serde_json::json!({"release": release})).map_err(display)?,
            &root.join("current.json"),
        )?;
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::symlink;
        let temporary = root.join(format!(".current-{}", std::process::id()));
        files::remove_path(&temporary)?;
        symlink(Path::new("releases").join(release), &temporary).map_err(display)?;
        fs::rename(temporary, root.join("current")).map_err(display)?;
    }
    Ok(())
}

#[cfg(windows)]
fn replace_file(source: &Path, destination: &Path) -> Result<(), String> {
    replace_bytes(fs::read(source).map_err(display)?, destination)
}

#[cfg(windows)]
fn replace_bytes(bytes: Vec<u8>, destination: &Path) -> Result<(), String> {
    let temporary = destination.with_extension(format!("tmp-{}", std::process::id()));
    fs::write(&temporary, bytes).map_err(display)?;
    fs::rename(temporary, destination).map_err(display)
}

fn active_daemon(root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        root.join("bin").join("gentd.exe")
    }
    #[cfg(not(windows))]
    {
        root.join("current").join("gentd")
    }
}

fn enable_updates(root: &Path, data_dir: &Path) -> Result<(), String> {
    let executable = active_cli(root);
    let status = Command::new(executable)
        .args(["update", "auto", "enable", "--data-dir"])
        .arg(data_dir)
        .status()
        .map_err(display)?;
    if status.success() {
        Ok(())
    } else {
        Err("Gent automatic updates could not be enabled".into())
    }
}

fn active_cli(root: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        root.join("bin").join("gent.exe")
    }
    #[cfg(not(windows))]
    {
        root.join("current").join("gent")
    }
}

fn required_files() -> &'static [&'static str] {
    #[cfg(windows)]
    {
        &[
            "gent.exe",
            "gentd.exe",
            "gent-launcher.exe",
            "gent-auto-update.ps1",
        ]
    }
    #[cfg(not(windows))]
    {
        &["gent", "gentd", "gent-auto-update.py"]
    }
}

fn default_root() -> PathBuf {
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

fn expected_target() -> Result<String, String> {
    if cfg!(all(target_os = "macos", target_arch = "aarch64")) {
        Ok("aarch64-apple-darwin".into())
    } else if cfg!(all(target_os = "macos", target_arch = "x86_64")) {
        Ok("x86_64-apple-darwin".into())
    } else if cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        Ok("x86_64-unknown-linux-gnu".into())
    } else if cfg!(all(windows, target_arch = "x86_64")) {
        Ok("x86_64-pc-windows-msvc".into())
    } else {
        Err("Gent bootstrap is unsupported on this platform".into())
    }
}

fn valid_version(value: &str) -> bool {
    value.strip_prefix('v').is_some_and(|value| {
        value.split('.').count() == 3
            && value
                .split('.')
                .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
    })
}
fn version_of(value: &str) -> Vec<u32> {
    value
        .trim_start_matches('v')
        .split('-')
        .next()
        .unwrap_or_default()
        .split('.')
        .filter_map(|part| part.parse().ok())
        .collect()
}
fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
#[path = "runtime_activation_tests.rs"]
mod tests;
