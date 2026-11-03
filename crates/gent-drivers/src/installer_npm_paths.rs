use std::{fs, path::Path};

use super::InstallerError;

pub(crate) fn require_supported_prefix(prefix: &Path) -> Result<(), InstallerError> {
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = prefix.components().next() {
        if matches!(
            prefix.kind(),
            std::path::Prefix::UNC(_, _) | std::path::Prefix::VerbatimUNC(_, _)
        ) {
            // Arborist's root walk does not terminate at a UNC share root. This
            // restriction is for the npm install prefix, not project/archive paths.
            return Err(InstallerError::UnsupportedPrefix);
        }
        if matches!(
            prefix.kind(),
            std::path::Prefix::Verbatim(_) | std::path::Prefix::DeviceNS(_)
        ) {
            return Err(InstallerError::InvalidArtifact);
        }
    }
    let _ = prefix;
    Ok(())
}

/// Keeps literal filesystem paths separate from npm's URL-like package specs.
/// Missing cache/config leaves are checked against the nearest existing ancestor.
pub(crate) fn checked_filesystem_path(path: &Path) -> Result<String, InstallerError> {
    let converted = filesystem_path(path);
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = path.components().next() {
        if matches!(
            prefix.kind(),
            std::path::Prefix::Verbatim(_) | std::path::Prefix::DeviceNS(_)
        ) {
            return Err(InstallerError::InvalidArtifact);
        }
    }
    let mut original = path.to_owned();
    let mut consumer = Path::new(&converted).to_owned();
    loop {
        match fs::canonicalize(&original) {
            Ok(canonical) => {
                let actual = fs::canonicalize(&consumer).map_err(io_error)?;
                return (canonical == actual)
                    .then_some(converted)
                    .ok_or(InstallerError::InvalidArtifact);
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let name = original
                    .file_name()
                    .ok_or(InstallerError::InvalidArtifact)?;
                if consumer.file_name() != Some(name) || !safe_missing_name(name) {
                    return Err(InstallerError::InvalidArtifact);
                }
                if !original.pop() || !consumer.pop() {
                    return Err(InstallerError::InvalidArtifact);
                }
            }
            Err(error) => return Err(io_error(error)),
        }
    }
}

fn safe_missing_name(name: &std::ffi::OsStr) -> bool {
    #[cfg(windows)]
    {
        let Some(name) = name.to_str() else {
            return false;
        };
        let base = name
            .split('.')
            .next()
            .unwrap_or_default()
            .to_ascii_uppercase();
        let numbered = base
            .strip_prefix("COM")
            .or_else(|| base.strip_prefix("LPT"));
        !name.ends_with(['.', ' '])
            && !name.contains(':')
            && !matches!(base.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            && !numbered.is_some_and(|suffix| {
                suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
            })
    }
    #[cfg(not(windows))]
    {
        let _ = name;
        true
    }
}

fn io_error(error: std::io::Error) -> InstallerError {
    InstallerError::Io(error.to_string())
}

// npm parses archive arguments as package specifications, where Windows verbatim
// prefixes are not filesystem syntax. Trust checks keep using the canonical path.
pub(crate) fn archive_argument(archive: &Path) -> String {
    let value = filesystem_path(archive);
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = Path::new(&value).components().next() {
        match prefix.kind() {
            std::path::Prefix::UNC(_, _) => {
                // npm resolves URL pathname only; four slashes retain the UNC host
                // in that pathname rather than dropping URL.hostname.
                return format!("file:////{}", npm_file_spec(&value[2..]).replace('\\', "/"));
            }
            std::path::Prefix::Disk(_) => return npm_file_spec(&value),
            _ => {}
        }
    }
    value
}

#[cfg(windows)]
fn npm_file_spec(value: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut encoded = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || b"-._~/:\\".contains(&byte) {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push(char::from(HEX[usize::from(byte >> 4)]));
            encoded.push(char::from(HEX[usize::from(byte & 15)]));
        }
    }
    encoded
}

pub(crate) fn filesystem_path(archive: &Path) -> String {
    let value = archive.to_string_lossy().into_owned();
    #[cfg(windows)]
    if let Some(std::path::Component::Prefix(prefix)) = archive.components().next() {
        match prefix.kind() {
            std::path::Prefix::VerbatimDisk(_) => return value[4..].to_owned(),
            std::path::Prefix::VerbatimUNC(_, _) => return format!(r"\\{}", &value[8..]),
            _ => {}
        }
    }
    value
}

#[cfg(test)]
#[path = "installer_npm_paths_tests.rs"]
mod tests;
