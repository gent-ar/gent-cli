//! Bounded signed source for uncomposed ordinary provider authority.
use crate::{
    claude_authority_preflight::ClaudeAuthorityPreflight,
    codex_authority_preflight::CodexAuthorityPreflight,
    compatibility_assessment::CompatibilityAssessment, node_runtime_lock::AppNodeRuntimeLock,
};
use ed25519_dalek::{Signature, Verifier, VerifyingKey};
use gent_adapters::{
    claude_authority_evidence::SignedClaudeAuthorityEvidence,
    codex_authority_evidence::SignedCodexAuthorityEvidence,
    compatibility::{SignedCompatibilityManifest, TrustedKeySet},
    compatibility_cache::CachedCompatibilityManifest,
    package_policy::{SignedPackagePolicy, VerifiedPackagePolicy},
};
use gent_ports::PackageInstallPolicy;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    path::Path,
};
const VERSION: u16 = 1;
const MAX_BYTES: u64 = 262_144;
const MAX_KEYS: usize = 8;
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct SignedOrdinaryAuthorityRelease {
    key_id: String,
    payload: OrdinaryAuthorityReleasePayload,
    signature_hex: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct OrdinaryAuthorityReleasePayload {
    version: u16,
    expires_at_unix_seconds: u64,
    revoked: bool,
    compatibility: SignedCompatibilityManifest,
    compatibility_keys: Vec<ReleaseVerificationKey>,
    package_policy: SignedPackagePolicy,
    package_policy_keys: Vec<ReleaseVerificationKey>,
    providers: Vec<ProviderAuthorityRelease>,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ReleaseVerificationKey {
    key_id: String,
    public_key_hex: String,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "provider", rename_all = "lowercase", deny_unknown_fields)]
enum ProviderAuthorityRelease {
    Claude {
        evidence: SignedClaudeAuthorityEvidence,
        evidence_keys: Vec<ReleaseVerificationKey>,
    },
    Codex {
        evidence: SignedCodexAuthorityEvidence,
        evidence_keys: Vec<ReleaseVerificationKey>,
    },
}
/// Material that has passed one root signature and every embedded authority fence.
#[derive(Clone, Debug)]
pub(crate) struct VerifiedOrdinaryAuthorityRelease {
    compatibility: CompatibilityAssessment,
    package_policy: VerifiedPackagePolicy,
    providers: Vec<VerifiedProviderAuthority>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum VerifiedProviderAuthority {
    Claude(ClaudeAuthorityPreflight),
    Codex(CodexAuthorityPreflight),
}
impl VerifiedOrdinaryAuthorityRelease {
    #[must_use]
    pub(crate) fn compatibility(&self) -> CompatibilityAssessment {
        self.compatibility.clone()
    }
    #[must_use]
    pub(crate) fn package_policy(&self) -> VerifiedPackagePolicy {
        self.package_policy.clone()
    }
    #[must_use]
    pub(crate) fn providers(&self) -> &[VerifiedProviderAuthority] {
        &self.providers
    }
}
#[derive(Debug, thiserror::Error)]
pub(crate) enum OrdinaryAuthorityReleaseError {
    #[error("ordinary authority release artifact is unavailable")]
    Unavailable,
    #[error("ordinary authority release artifact must be a regular non-symlink file")]
    NotRegular,
    #[error("ordinary authority release artifact exceeds the bounded size")]
    TooLarge,
    #[error("ordinary authority release artifact is malformed")]
    Malformed,
    #[error("ordinary authority release signer is not trusted")]
    UnknownSigner,
    #[error("ordinary authority release signature is invalid")]
    InvalidSignature,
    #[error("ordinary authority release is expired or revoked")]
    Inactive,
    #[error("ordinary authority release embedded authority is invalid")]
    EmbeddedAuthority,
}
impl SignedOrdinaryAuthorityRelease {
    /// Reads and fully validates one immutable release artifact against one root trust anchor.
    ///
    /// The root key is a bootstrap trust anchor, not a second mutable authority source. All
    /// provider scope, nested verification keys, compatibility, and npm policy come from this
    /// one signed artifact.
    pub(crate) fn load_bound(
        path: &Path,
        root_keys: &BTreeMap<String, VerifyingKey>,
        runtime: &AppNodeRuntimeLock,
        now: u64,
    ) -> Result<VerifiedOrdinaryAuthorityRelease, OrdinaryAuthorityReleaseError> {
        runtime
            .recheck()
            .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?;
        let bytes = read(path)?;
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|_| OrdinaryAuthorityReleaseError::Malformed)?;
        let release: Self = serde_json::from_value(value.clone())
            .map_err(|_| OrdinaryAuthorityReleaseError::Malformed)?;
        (serde_json::to_value(&release).ok() == Some(value))
            .then_some(())
            .ok_or(OrdinaryAuthorityReleaseError::Malformed)?;
        let root_key = root_keys
            .get(&release.key_id)
            .ok_or(OrdinaryAuthorityReleaseError::UnknownSigner)?;
        release.verify(root_key, runtime.node_digest_sha256(), now)
    }
    fn verify(
        self,
        root_key: &VerifyingKey,
        node_digest: &str,
        now: u64,
    ) -> Result<VerifiedOrdinaryAuthorityRelease, OrdinaryAuthorityReleaseError> {
        if self.signature_hex.len() != 128 || !valid_hex(&self.signature_hex) {
            return Err(OrdinaryAuthorityReleaseError::InvalidSignature);
        }
        let signature = Signature::from_slice(
            &hex::decode(&self.signature_hex)
                .map_err(|_| OrdinaryAuthorityReleaseError::InvalidSignature)?,
        )
        .map_err(|_| OrdinaryAuthorityReleaseError::InvalidSignature)?;
        root_key
            .verify(&canonical_payload(&self.payload)?, &signature)
            .map_err(|_| OrdinaryAuthorityReleaseError::InvalidSignature)?;
        if self.payload.version != VERSION
            || self.payload.revoked
            || self.payload.expires_at_unix_seconds < now
        {
            return Err(OrdinaryAuthorityReleaseError::Inactive);
        }
        let compatibility_keys = keys(&self.payload.compatibility_keys)?;
        let cached = CachedCompatibilityManifest::verify(
            self.payload.compatibility,
            &compatibility_keys,
            now,
        )
        .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?;
        let compatibility = CompatibilityAssessment::configured(compatibility_keys, cached, now);
        let policy = self
            .payload
            .package_policy
            .verify(&keys(&self.payload.package_policy_keys)?, now, node_digest)
            .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?;
        let mut providers = Vec::new();
        for provider in self.payload.providers {
            let verified = match provider {
                ProviderAuthorityRelease::Claude {
                    evidence,
                    evidence_keys,
                } => crate::claude_authority_preflight::verify(
                    &evidence,
                    &keys(&evidence_keys)?,
                    &compatibility,
                    now,
                )
                .map(VerifiedProviderAuthority::Claude)
                .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority),
                ProviderAuthorityRelease::Codex {
                    evidence,
                    evidence_keys,
                } => crate::codex_authority_preflight::verify(
                    &evidence,
                    &keys(&evidence_keys)?,
                    &compatibility,
                    now,
                )
                .map(VerifiedProviderAuthority::Codex)
                .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority),
            }?;
            let name = match verified {
                VerifiedProviderAuthority::Claude(_) => "claude",
                VerifiedProviderAuthority::Codex(_) => "codex",
            };
            policy
                .approved_package(name, now)
                .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?;
            if providers.contains(&verified) {
                return Err(OrdinaryAuthorityReleaseError::EmbeddedAuthority);
            }
            providers.push(verified);
        }
        (!providers.is_empty())
            .then_some(VerifiedOrdinaryAuthorityRelease {
                compatibility,
                package_policy: policy,
                providers,
            })
            .ok_or(OrdinaryAuthorityReleaseError::EmbeddedAuthority)
    }
}
fn read(path: &Path) -> Result<Vec<u8>, OrdinaryAuthorityReleaseError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| OrdinaryAuthorityReleaseError::Unavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(OrdinaryAuthorityReleaseError::NotRegular);
    }
    if metadata.len() > MAX_BYTES {
        return Err(OrdinaryAuthorityReleaseError::TooLarge);
    }
    let bytes = fs::read(path).map_err(|_| OrdinaryAuthorityReleaseError::Unavailable)?;
    (bytes.len() as u64 <= MAX_BYTES)
        .then_some(bytes)
        .ok_or(OrdinaryAuthorityReleaseError::TooLarge)
}
fn keys(values: &[ReleaseVerificationKey]) -> Result<TrustedKeySet, OrdinaryAuthorityReleaseError> {
    if values.is_empty() || values.len() > MAX_KEYS {
        return Err(OrdinaryAuthorityReleaseError::EmbeddedAuthority);
    }
    let mut ids = BTreeSet::new();
    let mut result = TrustedKeySet::default();
    for value in values {
        let bytes = hex::decode(&value.public_key_hex)
            .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?;
        if !valid_id(&value.key_id)
            || !valid_hex(&value.public_key_hex)
            || bytes.len() != 32
            || !ids.insert(&value.key_id)
        {
            return Err(OrdinaryAuthorityReleaseError::EmbeddedAuthority);
        }
        result.trust(
            &value.key_id,
            VerifyingKey::from_bytes(
                bytes
                    .as_slice()
                    .try_into()
                    .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?,
            )
            .map_err(|_| OrdinaryAuthorityReleaseError::EmbeddedAuthority)?,
        );
    }
    Ok(result)
}
fn valid_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
fn valid_hex(value: &str) -> bool {
    value
        .bytes()
        .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
fn canonical_payload(
    payload: &OrdinaryAuthorityReleasePayload,
) -> Result<Vec<u8>, OrdinaryAuthorityReleaseError> {
    serde_json::to_vec(
        &serde_json::to_value(payload).map_err(|_| OrdinaryAuthorityReleaseError::Malformed)?,
    )
    .map_err(|_| OrdinaryAuthorityReleaseError::Malformed)
}
#[cfg(test)]
#[path = "ordinary_authority_release_tests.rs"]
mod tests;
