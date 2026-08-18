//! Non-effectful loading for a future Claude-only authority composition.
//!
//! This validates an explicitly supplied, signed record. It does not discover, launch, or
//! communicate with Claude, and no daemon profile or capability calls this module.

use std::path::Path;

use gent_adapters::claude_authority_evidence::{
    ClaudeAuthorityEvidenceError, SignedClaudeAuthorityEvidence, VerifiedClaudeAuthorityEvidence,
};

use crate::{
    authority_evidence_input::{self, AuthorityEvidenceInputError},
    compatibility_assessment::CompatibilityAssessment,
};

/// An opaque, compatibility-bound record for a future Claude composition edge.
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ClaudeAuthorityPreflight {
    evidence: VerifiedClaudeAuthorityEvidence,
}

impl ClaudeAuthorityPreflight {
    /// Returns only the already verified provider-scoped evidence value.
    #[must_use]
    pub(crate) fn evidence(&self) -> &VerifiedClaudeAuthorityEvidence {
        &self.evidence
    }
}

/// Failure while reading or validating a future Claude authority input.
#[derive(Debug, thiserror::Error)]
pub(crate) enum ClaudeAuthorityPreflightError {
    #[error("Claude authority evidence record is unavailable")]
    RecordUnavailable,
    #[error("Claude authority evidence record must be a real regular file")]
    RecordNotRegular,
    #[error("Claude authority evidence record exceeds the bounded size")]
    RecordTooLarge,
    #[error("Claude authority evidence record is unreadable")]
    RecordUnreadable,
    #[error("Claude authority evidence record is not valid JSON")]
    RecordMalformed,
    #[error("Claude authority evidence requires at least one trusted key")]
    MissingTrustedKey,
    #[error("Claude authority key must be key-id:lowercase-hex")]
    InvalidKey,
    #[error("Claude authority key ids must be unique")]
    DuplicateKey,
    #[error("no verified signed compatibility manifest is available")]
    CompatibilityUnavailable,
    #[error("Claude authority evidence is pinned to a different compatibility manifest")]
    CompatibilityMismatch,
    #[error(transparent)]
    Evidence(#[from] ClaudeAuthorityEvidenceError),
}

impl From<AuthorityEvidenceInputError> for ClaudeAuthorityPreflightError {
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
/// Returns an error before any provider operation for malformed, expired, untrusted, or mismatched
/// evidence.
pub(crate) fn load(
    record_path: &Path,
    key_specs: &[String],
    compatibility: &CompatibilityAssessment,
    now_unix_seconds: u64,
) -> Result<ClaudeAuthorityPreflight, ClaudeAuthorityPreflightError> {
    let record: SignedClaudeAuthorityEvidence = authority_evidence_input::read_record(record_path)?;
    let keys = authority_evidence_input::parse_keys(key_specs)?;
    let evidence = record.verify(&keys, now_unix_seconds)?;
    let manifest = compatibility
        .manifest_sha256()
        .ok_or(ClaudeAuthorityPreflightError::CompatibilityUnavailable)?;
    if evidence.compatibility_manifest_sha256() != manifest {
        return Err(ClaudeAuthorityPreflightError::CompatibilityMismatch);
    }
    Ok(ClaudeAuthorityPreflight { evidence })
}

#[cfg(test)]
#[path = "claude_authority_preflight_tests.rs"]
mod tests;
