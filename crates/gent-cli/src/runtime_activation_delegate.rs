use gent_protocol::runtime_activation::ACTIVATION_MAINTENANCE_MAX_BYTES;
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    path::{Path, PathBuf},
};

pub(super) fn pinned_sibling(expected: &str) -> Result<PathBuf, String> {
    if expected.len() != 64 || !expected.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(super::error(
            "maintenanceIdentityRequired",
            "Invalid trusted daemon digest",
        ));
    }
    let current = std::env::current_exe().map_err(|e| e.to_string())?;
    let parent = current.parent().ok_or("CLI has no bundle directory")?;
    let canonical = parent.canonicalize().map_err(|e| e.to_string())?;
    let path = canonical.join(if cfg!(windows) { "gentd.exe" } else { "gentd" });
    verify(&path, expected)?;
    Ok(path)
}
pub(super) fn verify(path: &Path, expected: &str) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    #[cfg(windows)]
    let redirected = {
        use std::os::windows::fs::MetadataExt;
        metadata.file_attributes() & 0x400 != 0
    };
    #[cfg(not(windows))]
    let redirected = metadata.file_type().is_symlink();
    if !path.is_absolute()
        || !metadata.is_file()
        || redirected
        || path.canonicalize().map_err(|e| e.to_string())? != path
    {
        return Err("Maintenance daemon path is redirected".into());
    }
    let mut file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut digest = Sha256::new();
    let mut bytes = vec![0; 65536];
    loop {
        let n = file.read(&mut bytes).map_err(|e| e.to_string())?;
        if n == 0 {
            break;
        }
        digest.update(&bytes[..n]);
    }
    let actual = digest
        .finalize()
        .iter()
        .fold(String::new(), |mut hex, byte| {
            use std::fmt::Write;
            let _ = write!(hex, "{byte:02x}");
            hex
        });
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(super::error(
            "maintenanceIdentityMismatch",
            "Pinned daemon bytes changed",
        ));
    }
    Ok(())
}
pub(super) fn bounded_output(mut reader: impl Read) -> Result<Vec<u8>, String> {
    let mut output = Vec::new();
    let mut buffer = [0; 8192];
    let mut exceeded = false;
    loop {
        let count = reader.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        if output.len() + count <= ACTIVATION_MAINTENANCE_MAX_BYTES && !exceeded {
            output.extend_from_slice(&buffer[..count]);
        } else {
            exceeded = true;
        }
    }
    if exceeded {
        Err("Maintenance output exceeded its bound".into())
    } else {
        Ok(output)
    }
}
