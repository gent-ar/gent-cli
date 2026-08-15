//! Signed, expiring compatibility entries for public provider adapters.

use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use gent_types::RunVersionLock;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompatibilityEntry {
    pub id: String,
    pub provider: String,
    pub version: String,
    pub revoked: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CompatibilityManifest {
    pub manifest_version: u32,
    pub expires_at_unix_seconds: u64,
    pub entries: Vec<CompatibilityEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignedCompatibilityManifest {
    pub key_id: String,
    pub payload: CompatibilityManifest,
    pub signature_hex: String,
}

#[derive(Debug, thiserror::Error)]
pub enum CompatibilityError {
    #[error("compatibility manifest signature is malformed")]
    InvalidSignature,
    #[error("compatibility manifest signature does not verify")]
    SignatureMismatch,
    #[error("compatibility manifest has expired")]
    Expired,
    #[error("provider {provider} version {version} has no active compatible entry")]
    Unsupported { provider: String, version: String },
    #[error("compatibility manifest cannot be encoded: {0}")]
    Serialization(#[from] serde_json::Error),
}

impl SignedCompatibilityManifest {
    /// Verifies signature and expiry before a compatibility entry may be trusted.
    ///
    /// # Errors
    /// Returns an error for an invalid signature, expiry, revoked, or absent entry.
    pub fn verify_lock(
        &self,
        key: &VerifyingKey,
        lock: &RunVersionLock,
        now: u64,
    ) -> Result<(), CompatibilityError> {
        let payload = serde_json::to_vec(&self.payload)?;
        let encoded =
            hex::decode(&self.signature_hex).map_err(|_| CompatibilityError::InvalidSignature)?;
        let signature =
            Signature::from_slice(&encoded).map_err(|_| CompatibilityError::InvalidSignature)?;
        key.verify(&payload, &signature)
            .map_err(|_| CompatibilityError::SignatureMismatch)?;
        if self.payload.expires_at_unix_seconds < now {
            return Err(CompatibilityError::Expired);
        }
        let matched = self.payload.entries.iter().any(|entry| {
            entry.id == lock.compatibility_entry
                && entry.provider == lock.provider
                && entry.version == lock.version
                && !entry.revoked
        });
        matched
            .then_some(())
            .ok_or_else(|| CompatibilityError::Unsupported {
                provider: lock.provider.clone(),
                version: lock.version.clone(),
            })
    }
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use gent_types::RunVersionLock;

    use super::{CompatibilityEntry, CompatibilityManifest, SignedCompatibilityManifest};

    #[test]
    fn signed_entry_verifies_and_expired_entry_is_rejected() {
        let key = SigningKey::from_bytes(&[9; 32]);
        let payload = CompatibilityManifest {
            manifest_version: 1,
            expires_at_unix_seconds: 100,
            entries: vec![CompatibilityEntry {
                id: "claude-1".into(),
                provider: "claude".into(),
                version: "1.0".into(),
                revoked: false,
            }],
        };
        let signature_hex =
            hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes());
        let signed = SignedCompatibilityManifest {
            key_id: "test".into(),
            payload,
            signature_hex,
        };
        let lock = RunVersionLock {
            provider: "claude".into(),
            canonical_path: "/bin/claude".into(),
            file_identity: "x".into(),
            digest_sha256: "digest".into(),
            version: "1.0".into(),
            compatibility_entry: "claude-1".into(),
        };
        assert!(signed.verify_lock(&key.verifying_key(), &lock, 100).is_ok());
        assert!(
            signed
                .verify_lock(&key.verifying_key(), &lock, 101)
                .is_err()
        );
    }
}
