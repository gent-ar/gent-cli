use std::{
    collections::HashMap,
    fs,
    io::Read,
    path::{Path, PathBuf},
    sync::{Mutex, OnceLock, PoisonError},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use sha2::{Digest, Sha256};

const SETTLED: Duration = Duration::from_secs(2);

#[derive(Debug, thiserror::Error)]
pub enum ContentError {
    #[error("file changed while its content was verified")]
    Changed,
    #[error("cannot verify file content: {0}")]
    Io(#[from] std::io::Error),
}

pub struct VerifiedContent {
    pub canonical_path: PathBuf,
    pub metadata: fs::Metadata,
    pub sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Stamp {
    bytes: u64,
    modified: u128,
    changed: u128,
    volume: u64,
    index: u64,
}

type Verified = Mutex<HashMap<PathBuf, (Stamp, [u8; 32])>>;

fn verified() -> &'static Verified {
    static VERIFIED: OnceLock<Verified> = OnceLock::new();
    VERIFIED.get_or_init(Mutex::default)
}

pub fn sha256(path: &Path) -> Result<VerifiedContent, ContentError> {
    sha256_observed(path, |_| {})
}

pub(crate) fn sha256_observed(
    path: &Path,
    observed: impl FnMut(u64),
) -> Result<VerifiedContent, ContentError> {
    let canonical_path = fs::canonicalize(path)?;
    let file = open(&canonical_path)?;
    let metadata = file.metadata()?;
    if !metadata.is_file() {
        return Err(ContentError::Changed);
    }
    let before = stamp(&file, &metadata)?;
    let known = verified()
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&canonical_path)
        .filter(|(stamp, _)| *stamp == before)
        .map(|(_, sha256)| *sha256);
    if let Some(sha256) = known {
        return Ok(VerifiedContent {
            canonical_path,
            metadata,
            sha256,
        });
    }
    let started = SystemTime::now();
    let sha256 = stream(&file, metadata.len(), observed)?;
    let after = stamp(&file, &file.metadata()?)?;
    let reopened = open(&canonical_path)?;
    if after != before || stamp(&reopened, &reopened.metadata()?)? != before {
        return Err(ContentError::Changed);
    }
    if settled(before, started) {
        verified()
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(canonical_path.clone(), (before, sha256));
    }
    Ok(VerifiedContent {
        canonical_path,
        metadata,
        sha256,
    })
}

fn open(path: &Path) -> std::io::Result<fs::File> {
    let mut options = fs::OpenOptions::new();
    options.read(true);
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        options.share_mode(1);
    }
    options.open(path)
}

fn stream(
    mut file: &fs::File,
    expected: u64,
    mut observed: impl FnMut(u64),
) -> Result<[u8; 32], ContentError> {
    #[cfg(test)]
    tests::CONTENT_READS.with(|reads| reads.set(reads.get() + 1));
    let mut digest = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    let mut total = 0_u64;
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        digest.update(&buffer[..count]);
        total = total
            .checked_add(count as u64)
            .ok_or(ContentError::Changed)?;
        observed(total);
    }
    if total != expected {
        return Err(ContentError::Changed);
    }
    Ok(digest.finalize().into())
}

fn settled(stamp: Stamp, started: SystemTime) -> bool {
    let Ok(started) = started.duration_since(UNIX_EPOCH) else {
        return false;
    };
    let Some(boundary) = started.checked_sub(SETTLED) else {
        return false;
    };
    stamp.modified < boundary.as_nanos() && stamp.changed < boundary.as_nanos()
}

fn nanos(time: std::io::Result<SystemTime>) -> std::io::Result<u128> {
    Ok(time?
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_nanos()))
}

#[cfg(unix)]
fn stamp(_: &fs::File, metadata: &fs::Metadata) -> std::io::Result<Stamp> {
    use std::os::unix::fs::MetadataExt;
    let changed = i128::from(metadata.ctime()) * 1_000_000_000 + i128::from(metadata.ctime_nsec());
    Ok(Stamp {
        bytes: metadata.len(),
        modified: nanos(metadata.modified())?,
        changed: u128::try_from(changed).unwrap_or(0),
        volume: metadata.dev(),
        index: metadata.ino(),
    })
}

#[cfg(windows)]
fn stamp(file: &fs::File, metadata: &fs::Metadata) -> std::io::Result<Stamp> {
    let information = winapi_util::file::information(file)?;
    Ok(Stamp {
        bytes: information.file_size(),
        modified: nanos(metadata.modified())?,
        changed: nanos(metadata.created())?,
        volume: information.volume_serial_number(),
        index: information.file_index(),
    })
}

#[cfg(test)]
#[path = "verified_content_tests.rs"]
pub(crate) mod tests;
