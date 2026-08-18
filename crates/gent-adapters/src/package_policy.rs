//! Signed npm package policy for future approved public-provider hosts.

use ed25519_dalek::{Signature, Verifier};
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};
use serde::{Deserialize, Serialize};

use crate::compatibility::{CompatibilityError, TrustedKeySet};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackagePolicyEntry {
    pub provider: String,
    pub package_name: String,
    pub version: String,
    /// SRI digest of the exact tarball, verified by a future pack-and-install host.
    pub integrity: String,
    /// SHA-256 of the app-supplied Node binary allowed to fetch this package.
    pub node_runtime_digest_sha256: String,
    pub terms_version: String,
    pub revoked: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct PackagePolicy {
    pub policy_version: u32,
    pub expires_at_unix_seconds: u64,
    pub entries: Vec<PackagePolicyEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignedPackagePolicy {
    pub key_id: String,
    pub payload: PackagePolicy,
    pub signature_hex: String,
}

/// A signature-verified package policy bound to one immutable Node digest.
#[derive(Clone, Debug)]
pub struct VerifiedPackagePolicy {
    policy: SignedPackagePolicy,
    node_runtime_digest_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PackagePolicyError {
    #[error("package policy signature is malformed")]
    InvalidSignature,
    #[error("package policy signature does not verify")]
    SignatureMismatch,
    #[error("package policy has expired")]
    Expired,
    #[error("package policy signer is unavailable: {0}")]
    Trust(String),
    #[error("package policy cannot be encoded: {0}")]
    Serialization(String),
}

impl SignedPackagePolicy {
    /// Verifies the policy and binds it to the locked supplied Node runtime.
    ///
    /// # Errors
    /// Returns an error for bad signer trust, signature, or expiry.
    pub fn verify(
        &self,
        keys: &TrustedKeySet,
        now: u64,
        node_runtime_digest_sha256: impl Into<String>,
    ) -> Result<VerifiedPackagePolicy, PackagePolicyError> {
        let key = keys
            .key(&self.key_id)
            .map_err(|error| trust_error(&error))?;
        let payload = serde_json::to_vec(&self.payload)
            .map_err(|error| PackagePolicyError::Serialization(error.to_string()))?;
        let bytes =
            hex::decode(&self.signature_hex).map_err(|_| PackagePolicyError::InvalidSignature)?;
        let signature =
            Signature::from_slice(&bytes).map_err(|_| PackagePolicyError::InvalidSignature)?;
        key.verify(&payload, &signature)
            .map_err(|_| PackagePolicyError::SignatureMismatch)?;
        if self.payload.expires_at_unix_seconds < now {
            return Err(PackagePolicyError::Expired);
        }
        Ok(VerifiedPackagePolicy {
            policy: self.clone(),
            node_runtime_digest_sha256: node_runtime_digest_sha256.into(),
        })
    }
}

impl PackageInstallPolicy for VerifiedPackagePolicy {
    fn approved_package(
        &self,
        provider: &str,
        now_unix_seconds: u64,
    ) -> Result<ApprovedPackageInstall, PackageInstallPolicyError> {
        if self.policy.payload.expires_at_unix_seconds < now_unix_seconds {
            return Err(PackageInstallPolicyError::Rejected {
                provider: provider.into(),
                reason: "signed package policy expired".into(),
            });
        }
        let entry = self.policy.payload.entries.iter().find(|entry| {
            entry.provider == provider
                && entry.node_runtime_digest_sha256 == self.node_runtime_digest_sha256
                && !entry.revoked
        });
        entry
            .map(|entry| ApprovedPackageInstall {
                provider: entry.provider.clone(),
                package_name: entry.package_name.clone(),
                version: entry.version.clone(),
                integrity: entry.integrity.clone(),
            })
            .ok_or_else(|| PackageInstallPolicyError::Unavailable {
                provider: provider.into(),
            })
    }
}

fn trust_error(error: &CompatibilityError) -> PackagePolicyError {
    PackagePolicyError::Trust(error.to_string())
}

#[cfg(test)]
mod tests {
    use ed25519_dalek::{Signer, SigningKey};
    use gent_ports::PackageInstallPolicy;

    use super::{PackagePolicy, PackagePolicyEntry, SignedPackagePolicy, TrustedKeySet};

    #[test]
    fn verified_policy_selects_only_exact_nonrevoked_runtime_bound_package() {
        let key = SigningKey::from_bytes(&[4; 32]);
        let payload = PackagePolicy {
            policy_version: 1,
            expires_at_unix_seconds: 100,
            entries: vec![PackagePolicyEntry {
                provider: "codex".into(),
                package_name: "@openai/codex".into(),
                version: "0.147.0".into(),
                integrity: "sha512-test".into(),
                node_runtime_digest_sha256: "node-digest".into(),
                terms_version: "2026-01".into(),
                revoked: false,
            }],
        };
        let signed = SignedPackagePolicy {
            key_id: "test".into(),
            signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
            payload,
        };
        let mut keys = TrustedKeySet::default();
        keys.trust("test", key.verifying_key());
        let verified = signed.verify(&keys, 100, "node-digest").unwrap();
        assert_eq!(
            verified.approved_package("codex", 100).unwrap().selector(),
            "@openai/codex@0.147.0"
        );
        assert!(verified.approved_package("claude", 100).is_err());
        let wrong_node = signed.verify(&keys, 100, "other-node").unwrap();
        assert!(wrong_node.approved_package("codex", 100).is_err());
        assert!(verified.approved_package("codex", 101).is_err());
    }
}
