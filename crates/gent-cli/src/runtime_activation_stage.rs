use std::{fmt::Write, fs, path::Path};

use sha2::{Digest, Sha256};

use super::files;

pub(super) fn ensure_release(
    bootstrap: &Path,
    root: &Path,
    destination: &Path,
    release: &str,
    required: &[&str],
) -> Result<(), String> {
    if already_staged(bootstrap, destination, required)? {
        return Ok(());
    }
    let stage = root.join(format!(".stage-{release}-{}", std::process::id()));
    files::remove_path(&stage)?;
    files::copy_tree(bootstrap, &stage)?;
    if !destination.exists() {
        return fs::rename(&stage, destination).map_err(display);
    }
    let retired = root.join(format!(".retired-{release}-{}", std::process::id()));
    files::remove_path(&retired)?;
    fs::rename(destination, &retired).map_err(display)?;
    if let Err(error) = fs::rename(&stage, destination) {
        let _ = fs::rename(&retired, destination);
        return Err(display(error));
    }
    files::remove_path(&retired)
}

fn already_staged(bootstrap: &Path, destination: &Path, required: &[&str]) -> Result<bool, String> {
    if !destination.exists() {
        return Ok(false);
    }
    for name in required {
        let staged = digest(&destination.join(name))?;
        if staged.is_none() || staged != digest(&bootstrap.join(name))? {
            return Ok(false);
        }
    }
    Ok(true)
}

fn digest(path: &Path) -> Result<Option<String>, String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {
            Ok(Some(hex(&fs::read(path).map_err(display)?)))
        }
        Ok(_) => Ok(None),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(display(error)),
    }
}

fn hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}

fn display(error: impl std::fmt::Display) -> String {
    error.to_string()
}
