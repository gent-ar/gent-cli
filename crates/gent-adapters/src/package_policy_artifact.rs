//! Bounded read-only release artifact for a signed public-provider package policy.
//!
//! Gent does not refresh or rewrite this file during a prompt. A trusted release/update path may
//! place signed policy material, but every authority use re-reads and revalidates that artifact.

use std::{fs, path::Path};

use serde::{Deserialize, Serialize};

use crate::{
    compatibility::TrustedKeySet,
    package_policy::{PackagePolicyError, SignedPackagePolicy, VerifiedPackagePolicy},
};

const MAX_ARTIFACT_BYTES: u64 = 65_536;

/// A signed package-policy release artifact, not mutable runtime state.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct PackagePolicyArtifact {
    policy: SignedPackagePolicy,
}

/// Fail-closed errors while reading a package-policy release artifact.
#[derive(Debug, thiserror::Error)]
pub enum PackagePolicyArtifactError {
    #[error("package policy artifact is unavailable")]
    Unavailable,
    #[error("package policy artifact must be a regular non-symlink file")]
    NotRegular,
    #[error("package policy artifact exceeds the bounded size")]
    TooLarge,
    #[error("package policy artifact is unreadable")]
    Unreadable,
    #[error("package policy artifact is not valid strict JSON")]
    Malformed,
    #[error(transparent)]
    Policy(#[from] PackagePolicyError),
}

impl PackagePolicyArtifact {
    /// Verifies a signed policy before a trusted release path persists its artifact.
    ///
    /// # Errors
    /// Returns an error for invalid signer trust, signature, expiry, or policy shape.
    pub fn from_verified(
        policy: SignedPackagePolicy,
        keys: &TrustedKeySet,
        now_unix_seconds: u64,
    ) -> Result<Self, PackagePolicyArtifactError> {
        policy.verify_envelope(keys, now_unix_seconds)?;
        Ok(Self { policy })
    }

    /// Loads, revalidates, and binds a release artifact to the currently locked Node binary.
    ///
    /// # Errors
    /// Returns an error for unsafe paths, malformed artifact data, policy expiry/revocation, or
    /// an invalid current Node identity.
    pub fn load_bound(
        path: &Path,
        keys: &TrustedKeySet,
        now_unix_seconds: u64,
        node_runtime_digest_sha256: impl Into<String>,
    ) -> Result<VerifiedPackagePolicy, PackagePolicyArtifactError> {
        let artifact: Self = serde_json::from_slice(&read_bounded_regular(path)?)
            .map_err(|_| PackagePolicyArtifactError::Malformed)?;
        artifact
            .policy
            .verify(keys, now_unix_seconds, node_runtime_digest_sha256)
            .map_err(Into::into)
    }
}

fn read_bounded_regular(path: &Path) -> Result<Vec<u8>, PackagePolicyArtifactError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| PackagePolicyArtifactError::Unavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(PackagePolicyArtifactError::NotRegular);
    }
    if metadata.len() > MAX_ARTIFACT_BYTES {
        return Err(PackagePolicyArtifactError::TooLarge);
    }
    let bytes = fs::read(path).map_err(|_| PackagePolicyArtifactError::Unreadable)?;
    (bytes.len() as u64 <= MAX_ARTIFACT_BYTES)
        .then_some(bytes)
        .ok_or(PackagePolicyArtifactError::TooLarge)
}

#[cfg(test)]
#[path = "package_policy_artifact_tests.rs"]
mod tests;
