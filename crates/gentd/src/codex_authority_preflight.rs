//! Non-effectful loading for a future Codex-only authority composition.
//!
//! This module verifies an already supplied record only. It neither enables a daemon profile nor
//! discovers, launches, or communicates with a provider.

use gent_adapters::codex_authority_evidence::{
    CodexAuthorityEvidenceError, SignedCodexAuthorityEvidence, VerifiedCodexAuthorityEvidence,
};
use std::path::Path;

use crate::authority_evidence_input::{self, AuthorityEvidenceInputError};
use crate::compatibility_assessment::CompatibilityAssessment;

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

impl From<AuthorityEvidenceInputError> for CodexAuthorityPreflightError {
    fn from(value: AuthorityEvidenceInputError) -> Self {
        match value {
            AuthorityEvidenceInputError::Unavailable => Self::RecordUnavailable,
            AuthorityEvidenceInputError::NotRegular => Self::RecordNotRegular,
            AuthorityEvidenceInputError::TooLarge => Self::RecordTooLarge,
            AuthorityEvidenceInputError::Unreadable => Self::RecordUnreadable,
            AuthorityEvidenceInputError::Malformed => Self::RecordMalformed,
            AuthorityEvidenceInputError::MissingTrustedKey => Self::MissingTrustedKey,
            AuthorityEvidenceInputError::InvalidKey => Self::InvalidKey,
            AuthorityEvidenceInputError::DuplicateKey => Self::DuplicateKey,
        }
    }
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
    let record: SignedCodexAuthorityEvidence = authority_evidence_input::read_record(record_path)?;
    let keys = authority_evidence_input::parse_keys(key_specs)?;
    verify(&record, &keys, compatibility, now_unix_seconds)
}

/// Verifies an already-memory-resident Codex record from the one signed authority artifact.
pub(crate) fn verify(
    record: &SignedCodexAuthorityEvidence,
    keys: &gent_adapters::compatibility::TrustedKeySet,
    compatibility: &CompatibilityAssessment,
    now_unix_seconds: u64,
) -> Result<CodexAuthorityPreflight, CodexAuthorityPreflightError> {
    let evidence = record.verify(keys, now_unix_seconds)?;
    let manifest = compatibility
        .manifest_sha256()
        .ok_or(CodexAuthorityPreflightError::CompatibilityUnavailable)?;
    if evidence.compatibility_manifest_sha256() != manifest {
        return Err(CodexAuthorityPreflightError::CompatibilityMismatch);
    }
    Ok(CodexAuthorityPreflight { evidence })
}

#[cfg(test)]
#[path = "codex_authority_preflight_tests.rs"]
mod tests;
