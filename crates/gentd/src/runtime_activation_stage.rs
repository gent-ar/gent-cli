use std::{
    fmt::Write,
    fs,
    io::Read,
    path::{Path, PathBuf},
};

use sha2::{Digest, Sha256};

use super::files;

const MAX_FILES: usize = 20_000;
const MAX_BYTES: u64 = 8 * 1024 * 1024 * 1024;

pub(super) fn publish(
    bootstrap: &Path,
    root: &Path,
    destination: &Path,
    expected: &str,
) -> Result<(), String> {
    if fs::symlink_metadata(destination).is_ok() {
        return Err("immutable runtime generation already exists".into());
    }
    let temporary = tempfile::Builder::new()
        .prefix(".stage-")
        .tempdir_in(root)
        .map_err(super::display)?;
    files::copy_tree(bootstrap, temporary.path())?;
    if fingerprint(temporary.path())? != expected {
        return Err("runtime bootstrap changed while preparing its immutable generation".into());
    }
    fs::rename(temporary.path(), destination).map_err(super::display)
}

pub(super) fn fingerprint(root: &Path) -> Result<String, String> {
    let metadata = fs::symlink_metadata(root).map_err(super::display)?;
    if !metadata.is_dir() || files::redirected(&metadata) {
        return Err("runtime source is not a plain directory".into());
    }
    let mut entries = Vec::new();
    let mut bytes = 0;
    let mut visited = 0;
    collect(root, root, &mut entries, &mut bytes, &mut visited, 0)?;
    entries.sort_by(|left, right| left.0.cmp(&right.0));
    let mut tree = Sha256::new();
    tree.update(b"gent-runtime-tree-v1\0");
    for (name, path, permissions) in entries {
        tree.update((name.len() as u64).to_be_bytes());
        tree.update(name.as_bytes());
        tree.update(file_hash(&path)?);
        tree.update(permissions.to_be_bytes());
    }
    Ok(hex(&tree.finalize()))
}

pub(super) fn file_digest(path: &Path) -> Result<String, String> {
    file_hash(path).map(|hash| hex(&hash))
}

fn collect(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<(String, PathBuf, u32)>,
    bytes: &mut u64,
    visited: &mut usize,
    depth: usize,
) -> Result<(), String> {
    if depth > 128 {
        return Err("runtime source directory depth exceeds its verification bound".into());
    }
    for entry in fs::read_dir(directory).map_err(super::display)? {
        *visited += 1;
        if *visited > 40_000 {
            return Err("runtime source entry count exceeds its verification bound".into());
        }
        let path = entry.map_err(super::display)?.path();
        let metadata = fs::symlink_metadata(&path).map_err(super::display)?;
        if files::redirected(&metadata) {
            return Err("runtime source contains a redirected entry".into());
        }
        if metadata.is_dir() {
            collect(root, &path, entries, bytes, visited, depth + 1)?;
        } else if metadata.is_file() {
            *bytes = bytes
                .checked_add(metadata.len())
                .ok_or("runtime source size overflow")?;
            if entries.len() >= MAX_FILES || *bytes > MAX_BYTES {
                return Err("runtime source exceeds its verification bound".into());
            }
            let name = path
                .strip_prefix(root)
                .map_err(super::display)?
                .to_str()
                .ok_or("runtime source name is not UTF-8")?
                .replace('\\', "/");
            #[cfg(unix)]
            let permissions = {
                use std::os::unix::fs::PermissionsExt;
                let mode = metadata.permissions().mode();
                if mode & 0o7000 != 0 {
                    return Err("runtime source has unsupported special permissions".into());
                }
                mode & 0o777
            };
            #[cfg(not(unix))]
            let permissions = 0;
            entries.push((name, path, permissions));
        } else {
            return Err("runtime source contains an unsupported entry".into());
        }
    }
    Ok(())
}

fn file_hash(path: &Path) -> Result<[u8; 32], String> {
    let metadata = fs::symlink_metadata(path).map_err(super::display)?;
    if !metadata.is_file() || files::redirected(&metadata) {
        return Err("runtime file is missing or redirected".into());
    }
    let file = fs::File::open(path).map_err(super::display)?;
    file_hash_handle(&file)
}

pub(super) fn file_digest_handle(file: &fs::File) -> Result<String, String> {
    file_hash_handle(file).map(|hash| hex(&hash))
}

fn file_hash_handle(mut file: &fs::File) -> Result<[u8; 32], String> {
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 64 * 1024];
    let mut bytes: u64 = 0;
    loop {
        let read = file.read(&mut buffer).map_err(super::display)?;
        if read == 0 {
            break;
        }
        bytes = bytes
            .checked_add(read as u64)
            .ok_or("runtime file size overflow")?;
        if bytes > MAX_BYTES {
            return Err("runtime file exceeds its verification bound".into());
        }
        hash.update(&buffer[..read]);
    }
    Ok(hash.finalize().into())
}

fn hex(bytes: &[u8]) -> String {
    bytes
        .iter()
        .fold(String::with_capacity(64), |mut text, byte| {
            let _ = write!(text, "{byte:02x}");
            text
        })
}
