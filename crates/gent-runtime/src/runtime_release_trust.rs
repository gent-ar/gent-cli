//! Runtime-owned verification for signed Gent release metadata.

use std::collections::{BTreeMap, BTreeSet};

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use gent_types::{
    RUNTIME_RELEASE_INDEX_VERSION, RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseIndex,
    RuntimeReleaseManifest, RuntimeReleaseOffer, SignedRuntimeRelease, SignedRuntimeReleaseIndex,
};

/// Keyring and explicit signer-revocation state for runtime release verification.
#[derive(Clone, Debug, Default)]
pub struct RuntimeReleaseTrust {
    trusted_keys: BTreeMap<String, VerifyingKey>,
    revoked_keys: BTreeSet<String>,
}

/// Failure to establish runtime-release trust.
#[derive(Debug, thiserror::Error)]
pub enum RuntimeReleaseTrustError {
    #[error("runtime release signer is not trusted")]
    UnknownSigner,
    #[error("runtime release signer has been revoked")]
    RevokedSigner,
    #[error("runtime release signature is malformed or invalid")]
    InvalidSignature,
    #[error("runtime release manifest serialization failed")]
    Serialization(#[from] serde_json::Error),
    #[error("runtime release manifest has expired")]
    Expired,
    #[error("runtime release manifest has been revoked")]
    RevokedRelease,
    #[error("runtime release manifest version is unsupported")]
    UnsupportedManifestVersion,
    #[error("runtime release rollout percentage is invalid")]
    InvalidRollout,
    #[error("runtime release artifact metadata is invalid")]
    InvalidArtifact,
    #[error("runtime release compatibility range is invalid")]
    InvalidCompatibilityRange,
    #[error("runtime release artifact digest is not a SHA-256 hex digest")]
    InvalidDigest,
    #[error("runtime release index has expired or has been revoked")]
    InvalidIndex,
    #[error("runtime release index contains an invalid offer")]
    InvalidOffer,
}

impl RuntimeReleaseTrust {
    /// Builds a trust store from an explicit, public verification-key map.
    #[must_use]
    pub fn new(trusted_keys: BTreeMap<String, VerifyingKey>) -> Self {
        Self {
            trusted_keys,
            revoked_keys: BTreeSet::new(),
        }
    }

    /// Revokes a signer immediately, including previously cached releases.
    pub fn revoke_signer(&mut self, key_id: impl Into<String>) {
        self.revoked_keys.insert(key_id.into());
    }

    /// Verifies the envelope, signer, expiry, and safe manifest shape.
    ///
    /// # Errors
    /// Returns an error when a release cannot be trusted for staging or activation.
    pub fn verify_release(
        &self,
        release: &SignedRuntimeRelease,
        now_unix_seconds: u64,
    ) -> Result<(), RuntimeReleaseTrustError> {
        let key = self
            .trusted_keys
            .get(&release.key_id)
            .ok_or(RuntimeReleaseTrustError::UnknownSigner)?;
        if self.revoked_keys.contains(&release.key_id) {
            return Err(RuntimeReleaseTrustError::RevokedSigner);
        }
        verify_signed_payload(key, &release.payload, &release.signature_hex)?;
        validate_manifest(&release.payload, now_unix_seconds)
    }

    /// Verifies an expiring signed channel index before it is used for discovery.
    ///
    /// A verified index is not sufficient to stage an archive: its referenced
    /// [`SignedRuntimeRelease`] must pass [`Self::verify_release`] separately.
    ///
    /// # Errors
    /// Returns an error when the signer, signature, expiry, or offer shape is invalid.
    pub fn verify_index(
        &self,
        index: &SignedRuntimeReleaseIndex,
        now_unix_seconds: u64,
    ) -> Result<(), RuntimeReleaseTrustError> {
        let key = self
            .trusted_keys
            .get(&index.key_id)
            .ok_or(RuntimeReleaseTrustError::UnknownSigner)?;
        if self.revoked_keys.contains(&index.key_id) {
            return Err(RuntimeReleaseTrustError::RevokedSigner);
        }
        verify_signed_payload(key, &index.payload, &index.signature_hex)?;
        validate_index(&index.payload, now_unix_seconds)
    }
}

fn verify_signed_payload<T: serde::Serialize>(
    key: &VerifyingKey,
    payload: &T,
    signature_hex: &str,
) -> Result<(), RuntimeReleaseTrustError> {
    let bytes =
        hex::decode(signature_hex).map_err(|_| RuntimeReleaseTrustError::InvalidSignature)?;
    let signature =
        Signature::from_slice(&bytes).map_err(|_| RuntimeReleaseTrustError::InvalidSignature)?;
    let payload = serde_json::to_vec(payload)?;
    key.verify(&payload, &signature)
        .map_err(|_| RuntimeReleaseTrustError::InvalidSignature)
}

fn validate_index(
    index: &RuntimeReleaseIndex,
    now_unix_seconds: u64,
) -> Result<(), RuntimeReleaseTrustError> {
    if index.index_version != RUNTIME_RELEASE_INDEX_VERSION
        || index.expires_at_unix_seconds < now_unix_seconds
        || index.revoked
        || index.offers.is_empty()
    {
        return Err(RuntimeReleaseTrustError::InvalidIndex);
    }
    let mut identities = BTreeSet::new();
    for offer in &index.offers {
        validate_offer(offer)?;
        if !identities.insert((offer.channel as u8, offer.target.clone())) {
            return Err(RuntimeReleaseTrustError::InvalidOffer);
        }
    }
    Ok(())
}

fn validate_offer(offer: &RuntimeReleaseOffer) -> Result<(), RuntimeReleaseTrustError> {
    let numeric_tag = format!(
        "v{}.{}.{}",
        offer.release_version.major, offer.release_version.minor, offer.release_version.patch
    );
    let digest = &offer.manifest_digest_sha256;
    if offer.release_tag != numeric_tag
        || offer.target.trim().is_empty()
        || offer.manifest_name.trim().is_empty()
        || offer.manifest_name.contains('/')
        || digest.len() != 64
        || !digest.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(RuntimeReleaseTrustError::InvalidOffer);
    }
    Ok(())
}

fn validate_manifest(
    manifest: &RuntimeReleaseManifest,
    now_unix_seconds: u64,
) -> Result<(), RuntimeReleaseTrustError> {
    if manifest.manifest_version != RUNTIME_RELEASE_MANIFEST_VERSION {
        return Err(RuntimeReleaseTrustError::UnsupportedManifestVersion);
    }
    if manifest.expires_at_unix_seconds < now_unix_seconds {
        return Err(RuntimeReleaseTrustError::Expired);
    }
    if manifest.revoked {
        return Err(RuntimeReleaseTrustError::RevokedRelease);
    }
    if manifest.rollout_percent > 100 {
        return Err(RuntimeReleaseTrustError::InvalidRollout);
    }
    if manifest.protocol_min > manifest.protocol_max || manifest.schema_min > manifest.schema_max {
        return Err(RuntimeReleaseTrustError::InvalidCompatibilityRange);
    }
    if manifest.artifact.target.trim().is_empty()
        || manifest.artifact.archive_name.trim().is_empty()
        || manifest.artifact.size_bytes == 0
    {
        return Err(RuntimeReleaseTrustError::InvalidArtifact);
    }
    let digest = &manifest.artifact.digest_sha256;
    if digest.len() != 64 || !digest.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(RuntimeReleaseTrustError::InvalidDigest);
    }
    Ok(())
}

#[cfg(test)]
#[path = "runtime_release_trust_tests.rs"]
mod tests;
