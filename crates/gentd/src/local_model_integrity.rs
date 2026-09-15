use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    ffi::OsString,
    fs::{self, File, Metadata},
    io::{Read, Result},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

#[derive(Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct VerifiedFile {
    sha256: String,
    size_bytes: u64,
    modified_unix_nanos: u128,
    inode: u64,
    device: u64,
}

pub(crate) fn file_sha256(path: &Path) -> Result<String> {
    let mut file = File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buffer = [0_u8; 8_192];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            return Ok(hex::encode(hasher.finalize()));
        }
        hasher.update(&buffer[..count]);
    }
}

pub(crate) fn matches_sha256(path: &Path, expected: &str) -> Result<bool> {
    Ok(file_sha256(path)? == expected)
}

pub(crate) fn matches_verified_sha256(path: &Path, expected: &str) -> Result<bool> {
    let identity = fs::symlink_metadata(path)?;
    if retained(path).is_some_and(|verified| verified.proves(&identity, expected)) {
        return Ok(true);
    }
    if !matches_sha256(path, expected)? {
        let _ = fs::remove_file(verification_path(path));
        return Ok(false);
    }
    retain(path, &identity, expected);
    Ok(true)
}

pub(crate) fn remember_sha256(path: &Path, sha256: &str) {
    if let Ok(identity) = fs::symlink_metadata(path) {
        retain(path, &identity, sha256);
    }
}

impl VerifiedFile {
    fn capture(identity: &Metadata, sha256: &str) -> Option<Self> {
        if !identity.is_file() {
            return None;
        }
        let (inode, device) = file_identity(identity);
        Some(Self {
            sha256: sha256.to_owned(),
            size_bytes: identity.len(),
            modified_unix_nanos: identity
                .modified()
                .ok()?
                .duration_since(UNIX_EPOCH)
                .ok()?
                .as_nanos(),
            inode,
            device,
        })
    }

    fn proves(&self, identity: &Metadata, expected: &str) -> bool {
        Self::capture(identity, expected).is_some_and(|current| current == *self)
    }
}

fn verification_path(path: &Path) -> PathBuf {
    let mut name = OsString::from(path);
    name.push(".verified");
    PathBuf::from(name)
}

fn retained(path: &Path) -> Option<VerifiedFile> {
    serde_json::from_slice(&fs::read(verification_path(path)).ok()?).ok()
}

fn retain(path: &Path, identity: &Metadata, sha256: &str) {
    let Some(verified) = VerifiedFile::capture(identity, sha256) else {
        return;
    };
    if let Ok(encoded) = serde_json::to_vec(&verified) {
        let _ = fs::write(verification_path(path), encoded);
    }
}

#[cfg(unix)]
fn file_identity(identity: &Metadata) -> (u64, u64) {
    use std::os::unix::fs::MetadataExt;
    (identity.ino(), identity.dev())
}

#[cfg(not(unix))]
const fn file_identity(_: &Metadata) -> (u64, u64) {
    (0, 0)
}

#[cfg(test)]
#[path = "local_model_integrity_tests.rs"]
mod tests;
