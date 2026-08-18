//! Non-effectful loading for a future Codex-only authority composition.
//!
//! This module verifies an already supplied record only. It neither enables a daemon profile nor
//! discovers, launches, or communicates with a provider.

use std::{collections::BTreeSet, fs, path::Path};

use ed25519_dalek::VerifyingKey;
use gent_adapters::{
    codex_authority_evidence::{
        CodexAuthorityEvidenceError, SignedCodexAuthorityEvidence, VerifiedCodexAuthorityEvidence,
    },
    compatibility::TrustedKeySet,
};

use crate::compatibility_assessment::CompatibilityAssessment;

const MAX_RECORD_BYTES: u64 = 65_536;
const MAX_KEY_ID_BYTES: usize = 128;

/// An opaque, compatibility-bound record for a future Codex composition edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CodexAuthorityPreflight {
    evidence: VerifiedCodexAuthorityEvidence,
}

impl CodexAuthorityPreflight {
    /// Returns only the previously verified, provider-scoped evidence value.
    #[must_use]
    pub(crate) fn evidence(&self) -> &VerifiedCodexAuthorityEvidence {
        &self.evidence
    }
}

/// Failure while reading or validating a future Codex authority input.
#[derive(Debug, thiserror::Error)]
pub(crate) enum CodexAuthorityPreflightError {
    #[error("Codex authority evidence record is unavailable")]
    RecordUnavailable,
    #[error("Codex authority evidence record must be a real regular file")]
    RecordNotRegular,
    #[error("Codex authority evidence record exceeds the bounded size")]
    RecordTooLarge,
    #[error("Codex authority evidence record is unreadable")]
    RecordUnreadable,
    #[error("Codex authority evidence record is not valid JSON")]
    RecordMalformed,
    #[error("Codex authority evidence requires at least one trusted key")]
    MissingTrustedKey,
    #[error("Codex authority key must be key-id:lowercase-hex")]
    InvalidKey,
    #[error("Codex authority key ids must be unique")]
    DuplicateKey,
    #[error("no verified signed compatibility manifest is available")]
    CompatibilityUnavailable,
    #[error("Codex authority evidence is pinned to a different compatibility manifest")]
    CompatibilityMismatch,
    #[error(transparent)]
    Evidence(#[from] CodexAuthorityEvidenceError),
}

/// Loads one bounded signed record and binds it to the exact loaded compatibility envelope.
///
/// # Errors
/// Returns an error before any provider operation for malformed input, untrusted evidence, or a
/// missing/mismatched signed compatibility cache.
pub(crate) fn load(
    record_path: &Path,
    key_specs: &[String],
    compatibility: &CompatibilityAssessment,
    now_unix_seconds: u64,
) -> Result<CodexAuthorityPreflight, CodexAuthorityPreflightError> {
    let record = read_record(record_path)?;
    let keys = parse_keys(key_specs)?;
    let evidence = record.verify(&keys, now_unix_seconds)?;
    let manifest = compatibility
        .manifest_sha256()
        .ok_or(CodexAuthorityPreflightError::CompatibilityUnavailable)?;
    if evidence.compatibility_manifest_sha256() != manifest {
        return Err(CodexAuthorityPreflightError::CompatibilityMismatch);
    }
    Ok(CodexAuthorityPreflight { evidence })
}

fn read_record(path: &Path) -> Result<SignedCodexAuthorityEvidence, CodexAuthorityPreflightError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|_| CodexAuthorityPreflightError::RecordUnavailable)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(CodexAuthorityPreflightError::RecordNotRegular);
    }
    if metadata.len() > MAX_RECORD_BYTES {
        return Err(CodexAuthorityPreflightError::RecordTooLarge);
    }
    let bytes = fs::read(path).map_err(|_| CodexAuthorityPreflightError::RecordUnreadable)?;
    if bytes.len() as u64 > MAX_RECORD_BYTES {
        return Err(CodexAuthorityPreflightError::RecordTooLarge);
    }
    serde_json::from_slice(&bytes).map_err(|_| CodexAuthorityPreflightError::RecordMalformed)
}

fn parse_keys(values: &[String]) -> Result<TrustedKeySet, CodexAuthorityPreflightError> {
    if values.is_empty() {
        return Err(CodexAuthorityPreflightError::MissingTrustedKey);
    }
    let mut ids = BTreeSet::new();
    let mut keys = TrustedKeySet::default();
    for value in values {
        let (key_id, encoded) = value
            .split_once(':')
            .ok_or(CodexAuthorityPreflightError::InvalidKey)?;
        if !valid_key_id(key_id)
            || encoded.len() != 64
            || !encoded
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(CodexAuthorityPreflightError::InvalidKey);
        }
        if !ids.insert(key_id) {
            return Err(CodexAuthorityPreflightError::DuplicateKey);
        }
        let bytes = hex::decode(encoded).map_err(|_| CodexAuthorityPreflightError::InvalidKey)?;
        let key = VerifyingKey::from_bytes(
            bytes
                .as_slice()
                .try_into()
                .map_err(|_| CodexAuthorityPreflightError::InvalidKey)?,
        )
        .map_err(|_| CodexAuthorityPreflightError::InvalidKey)?;
        keys.trust(key_id, key);
    }
    Ok(keys)
}

fn valid_key_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_KEY_ID_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

#[cfg(test)]
#[path = "codex_authority_preflight_tests.rs"]
mod tests;
