//! Native SDK companion aliases, created only in a fresh unpublished npm generation.
use std::os::windows::{
    ffi::OsStrExt,
    fs::{MetadataExt, OpenOptionsExt},
};
use std::{
    fs::{self, File, OpenOptions},
    io::Read,
    path::Path,
};

const HELPERS: [&str; 2] = [
    "codex-command-runner.exe",
    "codex-windows-sandbox-setup.exe",
];

pub(super) fn prepare(executable: &Path) -> Result<Vec<File>, String> {
    let bin = executable.parent().ok_or("missing Codex bin directory")?;
    let root = bin.parent().ok_or("missing Codex native root")?;
    let resources = root.join("codex-resources");
    // Older approved SDK packages may not carry the native sandbox companions.
    if !resources
        .try_exists()
        .map_err(|_| "cannot inspect Codex resources")?
    {
        return Ok(Vec::new());
    }
    plain(&resources, true)?;
    let mut pins = Vec::new();
    for name in HELPERS {
        let source = resources.join(name);
        let target = bin.join(name);
        let dos = target.as_os_str().encode_wide().collect::<Vec<_>>();
        let length = if target.to_string_lossy().starts_with(r"\\?\") {
            dos.len() - 4
        } else {
            dos.len()
        };
        if length >= 260 {
            return Err("Codex companion launch path exceeds Windows MAX_PATH".into());
        }
        plain(&source, false)?;
        let mut source_pin = pin(&source)?;
        match fs::hard_link(&source, &target) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(_) => return Err("cannot materialize Codex native companion".into()),
        }
        plain(&target, false)?;
        let mut target_pin = pin(&target)?;
        let mut original = Vec::new();
        let mut alias = Vec::new();
        source_pin
            .read_to_end(&mut original)
            .map_err(|_| "cannot read Codex companion")?;
        target_pin
            .read_to_end(&mut alias)
            .map_err(|_| "cannot verify Codex companion alias")?;
        if original != alias {
            return Err("Codex companion alias differs from approved package".into());
        }
        pins.push(source_pin);
        pins.push(target_pin);
    }
    Ok(pins)
}

fn pin(path: &Path) -> Result<File, String> {
    OpenOptions::new()
        .read(true)
        .share_mode(1)
        .open(path)
        .map_err(|_| "cannot pin Codex companion".into())
}

fn plain(path: &Path, directory: bool) -> Result<(), String> {
    let m = fs::symlink_metadata(path).map_err(|_| "Codex companion is missing")?;
    if m.file_attributes() & 0x400 != 0 || m.is_dir() != directory || (!directory && !m.is_file()) {
        return Err("Codex companion path is not an ordinary package entry".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "windows_codex_companions_tests.rs"]
mod tests;
