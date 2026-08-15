//! Verified offline cache for public compatibility manifests.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::compatibility::{CompatibilityError, SignedCompatibilityManifest, TrustedKeySet};

/// A manifest that was verified before being written to an offline cache.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CachedCompatibilityManifest {
    pub verified_at_unix_seconds: u64,
    pub manifest: SignedCompatibilityManifest,
}

impl CachedCompatibilityManifest {
    /// Verifies a downloaded manifest before it may become cached offline state.
    ///
    /// # Errors
    /// Returns an error for untrusted, revoked, invalid, or expired manifests.
    pub fn verify(
        manifest: SignedCompatibilityManifest,
        keys: &TrustedKeySet,
        now: u64,
    ) -> Result<Self, CompatibilityError> {
        keys.verify_manifest(&manifest, now)?;
        Ok(Self {
            verified_at_unix_seconds: now,
            manifest,
        })
    }

    /// Rechecks signature, revocation, and expiry before an offline launch.
    ///
    /// # Errors
    /// Returns an error when cached data is no longer trustworthy.
    pub fn revalidate(&self, keys: &TrustedKeySet, now: u64) -> Result<(), CompatibilityError> {
        keys.verify_manifest(&self.manifest, now)
    }

    /// Atomically writes already verified cache content to a caller-selected file.
    ///
    /// # Errors
    /// Returns an error when serialization, write, or rename fails.
    pub fn store(&self, path: &Path) -> Result<(), CacheError> {
        let bytes = serde_json::to_vec(self)?;
        let temporary = path.with_extension("tmp");
        std::fs::write(&temporary, bytes)?;
        std::fs::rename(temporary, path)?;
        Ok(())
    }

    /// Loads and revalidates cached state; expired or revoked data is never returned.
    ///
    /// # Errors
    /// Returns an error when cache data is unreadable or no longer trustworthy.
    pub fn load(path: &Path, keys: &TrustedKeySet, now: u64) -> Result<Self, CacheError> {
        let bytes = std::fs::read(path)?;
        let cached = serde_json::from_slice::<Self>(&bytes)?;
        cached.revalidate(keys, now)?;
        Ok(cached)
    }
}

#[derive(Debug, thiserror::Error)]
pub enum CacheError {
    #[error(transparent)]
    Compatibility(#[from] CompatibilityError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Serialization(#[from] serde_json::Error),
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};

    use super::CachedCompatibilityManifest;
    use crate::compatibility::{CompatibilityManifest, SignedCompatibilityManifest, TrustedKeySet};

    fn signed() -> (SigningKey, SignedCompatibilityManifest) {
        let key = SigningKey::from_bytes(&[4; 32]);
        let payload = CompatibilityManifest {
            manifest_version: 1,
            expires_at_unix_seconds: 20,
            entries: Vec::new(),
        };
        let signature_hex =
            hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes());
        (
            key,
            SignedCompatibilityManifest {
                key_id: "main".into(),
                payload,
                signature_hex,
            },
        )
    }

    #[test]
    fn offline_cache_revalidates_expiry_and_revocation() {
        let (key, manifest) = signed();
        let mut keys = TrustedKeySet::default();
        keys.trust("main", key.verifying_key());
        let cached = CachedCompatibilityManifest::verify(manifest, &keys, 10).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("compatibility.json");
        cached.store(&path).unwrap();
        assert!(CachedCompatibilityManifest::load(&path, &keys, 20).is_ok());
        assert!(CachedCompatibilityManifest::load(&path, &keys, 21).is_err());
        keys.revoke("main");
        assert!(CachedCompatibilityManifest::load(&path, &keys, 20).is_err());
    }
}
