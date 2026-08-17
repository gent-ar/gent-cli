//! Verified offline cache for runtime release metadata.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use gent_types::SignedRuntimeRelease;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{RuntimeReleaseTrust, RuntimeReleaseTrustError};

/// A signed release that was verified before it became offline state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CachedRuntimeRelease {
    verified_at_unix_seconds: u64,
    release: SignedRuntimeRelease,
}

/// Failure to read, revalidate, or atomically store a release cache.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeReleaseCacheError {
    #[error(transparent)]
    Trust(#[from] RuntimeReleaseTrustError),
    #[error("runtime release cache serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("runtime release cache I/O failed")]
    Io(#[from] std::io::Error),
    #[error("runtime release cache path must include a file name")]
    InvalidPath,
}

impl CachedRuntimeRelease {
    /// Verifies a remote release before it is eligible for offline storage.
    ///
    /// # Errors
    /// Returns an error if the signed release is untrusted or unsafe.
    pub fn verify(
        release: SignedRuntimeRelease,
        trust: &RuntimeReleaseTrust,
        now_unix_seconds: u64,
    ) -> Result<Self, RuntimeReleaseCacheError> {
        trust.verify_release(&release, now_unix_seconds)?;
        Ok(Self {
            verified_at_unix_seconds: now_unix_seconds,
            release,
        })
    }

    /// Revalidates a cached release at the moment it would be used.
    ///
    /// # Errors
    /// Returns an error when revocation, expiry, or signature invalidates the cache.
    pub fn revalidate(
        &self,
        trust: &RuntimeReleaseTrust,
        now_unix_seconds: u64,
    ) -> Result<(), RuntimeReleaseCacheError> {
        trust.verify_release(&self.release, now_unix_seconds)?;
        Ok(())
    }

    /// Atomically writes a still-trusted cache entry to an exact caller-selected path.
    ///
    /// # Errors
    /// Returns an error when revalidation, serialization, or the atomic write fails.
    pub fn store(
        &self,
        path: &Path,
        trust: &RuntimeReleaseTrust,
        now_unix_seconds: u64,
    ) -> Result<(), RuntimeReleaseCacheError> {
        self.revalidate(trust, now_unix_seconds)?;
        write_atomically(path, &serde_json::to_vec(self)?)
    }

    /// Loads and revalidates a cached release; stale offline state is never returned.
    ///
    /// # Errors
    /// Returns an error when cache data is unreadable or cannot still be trusted.
    pub fn load(
        path: &Path,
        trust: &RuntimeReleaseTrust,
        now_unix_seconds: u64,
    ) -> Result<Self, RuntimeReleaseCacheError> {
        let metadata = fs::symlink_metadata(path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return Err(RuntimeReleaseCacheError::InvalidPath);
        }
        let cached: Self = serde_json::from_slice(&fs::read(path)?)?;
        cached.revalidate(trust, now_unix_seconds)?;
        Ok(cached)
    }

    /// Returns the signed release without bypassing cache revalidation at call sites.
    #[must_use]
    pub const fn release(&self) -> &SignedRuntimeRelease {
        &self.release
    }

    /// Returns the time when this cache entry was last independently verified.
    #[must_use]
    pub const fn verified_at_unix_seconds(&self) -> u64 {
        self.verified_at_unix_seconds
    }
}

fn write_atomically(path: &Path, bytes: &[u8]) -> Result<(), RuntimeReleaseCacheError> {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or(RuntimeReleaseCacheError::InvalidPath)?;
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let metadata = fs::symlink_metadata(parent)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(RuntimeReleaseCacheError::InvalidPath);
    }
    let temporary = parent.join(format!(".{file_name}.{}.tmp", Uuid::new_v4()));
    let write_result = (|| -> Result<(), std::io::Error> {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if write_result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    write_result.map_err(RuntimeReleaseCacheError::Io)
}

#[cfg(test)]
#[path = "runtime_release_cache_tests.rs"]
mod tests;
