use super::mcp::Error;
use std::{
    fs::{File, OpenOptions},
    path::Path,
};
pub(super) fn overlap(a: &Path, b: &Path) -> bool {
    let a = a.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    let b = b.to_string_lossy().replace('/', "\\").to_ascii_lowercase();
    let a = a.trim_end_matches('\\');
    let b = b.trim_end_matches('\\');
    a == b
        || a.strip_prefix(b).is_some_and(|s| s.starts_with('\\'))
        || b.strip_prefix(a).is_some_and(|s| s.starts_with('\\'))
}
pub(super) fn pin(
    path: &Path,
    workspace: &Path,
    worker_root: &Path,
) -> Result<(std::path::PathBuf, Vec<File>), Error> {
    let canonical = path.canonicalize().map_err(|_| Error::Protocol)?;
    if overlap(&canonical, workspace) || overlap(&canonical, worker_root) {
        return Err(Error::Denied);
    }
    let mut options = OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1).custom_flags(0x0020_0000);
    }
    let handle = options.open(&canonical).map_err(|_| Error::Protocol)?;
    if !handle.metadata().map_err(|_| Error::Protocol)?.is_file() {
        return Err(Error::Protocol);
    }
    let mut pins = vec![handle];
    #[cfg(windows)]
    {
        use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
        for parent in canonical.ancestors().skip(1) {
            let mut options = OpenOptions::new();
            options.read(true).share_mode(3).custom_flags(0x0220_0000);
            let lock = options.open(parent).map_err(|_| Error::Denied)?;
            if lock
                .metadata()
                .map_err(|_| Error::Denied)?
                .file_attributes()
                & 0x400
                != 0
            {
                return Err(Error::Denied);
            }
            pins.push(lock);
        }
    }
    Ok((canonical, pins))
}
pub(super) fn host_data(
    path: &Path,
    workspace: &Path,
    worker_root: &Path,
) -> Result<(std::path::PathBuf, Vec<File>), Error> {
    let canonical = path.canonicalize().map_err(|_| Error::Protocol)?;
    let worker_root = worker_root.canonicalize().map_err(|_| Error::Denied)?;
    // The normal host data directory owns provider-state/<private worker root>.
    // It is an MCP CLI argument, never a filesystem grant to the worker.
    // Reject data inside worker-writable state, but permit its trusted host ancestor.
    let data_text = canonical
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let worker_text = worker_root
        .to_string_lossy()
        .replace('/', "\\")
        .to_ascii_lowercase();
    let worker_text = worker_text.trim_end_matches('\\');
    let worker_contains_data = data_text == worker_text
        || data_text
            .strip_prefix(worker_text)
            .is_some_and(|tail| tail.starts_with('\\'));
    if !canonical.is_dir() || overlap(&canonical, workspace) || worker_contains_data {
        return Err(Error::Denied);
    }
    let mut locks = Vec::new();
    for parent in canonical.ancestors() {
        let mut options = OpenOptions::new();
        options.read(true);
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            options.share_mode(3).custom_flags(0x0220_0000);
        }
        locks.push(options.open(parent).map_err(|_| Error::Denied)?);
    }
    // IPC hashes the caller's data-directory spelling. Canonical Win32 extended
    // paths are valid file authority but are not an ordinary IPC client alias.
    // Keep all physical locks; pass the ordinary spelling of that SAME target.
    #[cfg(windows)]
    let canonical = ordinary_argument(&canonical)?;
    Ok((canonical, locks))
}

#[cfg(windows)]
pub(crate) fn ordinary_argument(canonical: &Path) -> Result<std::path::PathBuf, Error> {
    let value = canonical.to_str().ok_or(Error::Protocol)?;
    let ordinary = if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
        std::path::PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = value.strip_prefix(r"\\?\") {
        let bytes = rest.as_bytes();
        if bytes.len() < 3
            || !bytes[0].is_ascii_alphabetic()
            || bytes[1] != b':'
            || bytes[2] != b'\\'
        {
            return Err(Error::Protocol);
        }
        rest.into()
    } else {
        canonical.to_path_buf()
    };
    let checked = ordinary.canonicalize().map_err(|_| Error::Protocol)?;
    if !checked
        .as_os_str()
        .eq_ignore_ascii_case(canonical.as_os_str())
    {
        return Err(Error::Denied);
    }
    Ok(ordinary)
}

#[cfg(test)]
#[path = "provider_mcp_paths_tests.rs"]
mod tests;
