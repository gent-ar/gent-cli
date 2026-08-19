//! Signed npm package policy for approved public-provider hosts.

use std::collections::BTreeSet;

use base64::{Engine, engine::general_purpose::STANDARD};
use ed25519_dalek::{Signature, Verifier};
use gent_ports::{ApprovedPackageInstall, PackageInstallPolicy, PackageInstallPolicyError};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::compatibility::{CompatibilityError, TrustedKeySet};

const POLICY_VERSION: u32 = 1;
const MAX_ENTRIES: usize = 16;
const MAX_TEXT_BYTES: usize = 128;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackagePolicyEntry {
    pub provider: String,
    pub package_name: String,
    pub version: String,
    /// SRI digest of the exact tarball, verified again by the package installer.
    pub integrity: String,
    /// SHA-256 of the app-supplied Node binary allowed to fetch this package.
    pub node_runtime_digest_sha256: String,
    pub terms_version: String,
    pub revoked: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackagePolicy {
    pub policy_version: u32,
    pub expires_at_unix_seconds: u64,
    pub entries: Vec<PackagePolicyEntry>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
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
    policy_digest_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, thiserror::Error)]
pub enum PackagePolicyError {
    #[error("package policy has an invalid shape")]
    InvalidShape,
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
    /// Verifies signer, signature, expiry, and the bounded immutable policy shape.
    ///
    /// # Errors
    /// Returns an error for malformed, expired, untrusted, or ambiguous policy data.
    pub fn verify_envelope(
        &self,
        keys: &TrustedKeySet,
        now: u64,
    ) -> Result<(), PackagePolicyError> {
        validate(self, now)?;
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
            .map_err(|_| PackagePolicyError::SignatureMismatch)
    }

    /// Verifies the policy and binds it to the locked supplied Node runtime.
    ///
    /// # Errors
    /// Returns an error for bad signer trust, signature, expiry, or Node identity.
    pub fn verify(
        &self,
        keys: &TrustedKeySet,
        now: u64,
        node_runtime_digest_sha256: impl Into<String>,
    ) -> Result<VerifiedPackagePolicy, PackagePolicyError> {
        self.verify_envelope(keys, now)?;
        bind_node(self, node_runtime_digest_sha256.into())
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
                package_policy_digest_sha256: self.policy_digest_sha256.clone(),
            })
            .ok_or_else(|| PackageInstallPolicyError::Unavailable {
                provider: provider.into(),
            })
    }
}

fn validate(policy: &SignedPackagePolicy, now: u64) -> Result<(), PackagePolicyError> {
    if !valid_text(&policy.key_id) || !valid_signature(&policy.signature_hex) {
        return Err(PackagePolicyError::InvalidShape);
    }
    let payload = &policy.payload;
    if payload.policy_version != POLICY_VERSION || payload.expires_at_unix_seconds < now {
        return Err(if payload.expires_at_unix_seconds < now {
            PackagePolicyError::Expired
        } else {
            PackagePolicyError::InvalidShape
        });
    }
    if payload.entries.is_empty() || payload.entries.len() > MAX_ENTRIES {
        return Err(PackagePolicyError::InvalidShape);
    }
    let mut identities = BTreeSet::new();
    for entry in &payload.entries {
        if !valid_entry(entry)
            || !identities.insert((
                entry.provider.as_str(),
                entry.node_runtime_digest_sha256.as_str(),
            ))
        {
            return Err(PackagePolicyError::InvalidShape);
        }
    }
    Ok(())
}

fn bind_node(
    policy: &SignedPackagePolicy,
    node_runtime_digest_sha256: String,
) -> Result<VerifiedPackagePolicy, PackagePolicyError> {
    if !valid_sha256(&node_runtime_digest_sha256) {
        return Err(PackagePolicyError::InvalidShape);
    }
    Ok(VerifiedPackagePolicy {
        policy: policy.clone(),
        node_runtime_digest_sha256,
        policy_digest_sha256: digest_policy(policy)?,
    })
}

fn valid_entry(entry: &PackagePolicyEntry) -> bool {
    expected_package(&entry.provider) == Some(entry.package_name.as_str())
        && valid_version(&entry.version)
        && valid_sri(&entry.integrity)
        && valid_sha256(&entry.node_runtime_digest_sha256)
        && valid_text(&entry.terms_version)
}

fn expected_package(provider: &str) -> Option<&'static str> {
    match provider {
        "claude" => Some("@anthropic-ai/claude-code"),
        "codex" => Some("@openai/codex"),
        _ => None,
    }
}

fn valid_version(value: &str) -> bool {
    let (core, prerelease) = value
        .split_once('-')
        .map_or((value, None), |parts| (parts.0, Some(parts.1)));
    core.split('.').count() == 3
        && core.split('.').all(valid_numeric_component)
        && prerelease.is_none_or(|suffix| {
            !suffix.is_empty()
                && suffix.len() <= MAX_TEXT_BYTES
                && suffix
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'-'))
        })
}

fn valid_numeric_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 10
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
}

fn valid_sri(value: &str) -> bool {
    let Some(encoded) = value.strip_prefix("sha512-") else {
        return false;
    };
    let Ok(digest) = STANDARD.decode(encoded) else {
        return false;
    };
    digest.len() == 64 && STANDARD.encode(digest) == encoded
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn valid_text(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_TEXT_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn valid_signature(value: &str) -> bool {
    value.len() == 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn digest_policy(policy: &SignedPackagePolicy) -> Result<String, PackagePolicyError> {
    let bytes = serde_json::to_vec(policy)
        .map_err(|error| PackagePolicyError::Serialization(error.to_string()))?;
    Ok(hex::encode(Sha256::digest(bytes)))
}

fn trust_error(error: &CompatibilityError) -> PackagePolicyError {
    PackagePolicyError::Trust(error.to_string())
}

#[cfg(test)]
#[path = "package_policy_tests.rs"]
mod tests;
