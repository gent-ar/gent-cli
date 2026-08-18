//! Signed, provider-scoped Claude evidence for a future authority composition.
//!
//! Verification is deliberately metadata-only: it neither reads an artifact nor starts Claude.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Verifier};
use serde::{Deserialize, Serialize};

use crate::compatibility::{CompatibilityError, TrustedKeySet};

const SCHEMA_VERSION: u32 = 1;
const MAX_TEXT_BYTES: usize = 128;
const MAX_PLATFORM_BYTES: usize = 64;

/// The complete reviewed Claude scenario set required for live authority.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeEvidenceScenario {
    FullTurn,
    ToolUse,
    ToolError,
    Thinking,
    PermissionPrompt,
    PermissionPersistent,
    PlanMode,
    Subagent,
    Compaction,
    McpTool,
    Resume,
    Interrupt,
    Steer,
    UsageCost,
    MalformedTolerance,
}

const REQUIRED_SCENARIOS: [ClaudeEvidenceScenario; 15] = [
    ClaudeEvidenceScenario::FullTurn,
    ClaudeEvidenceScenario::ToolUse,
    ClaudeEvidenceScenario::ToolError,
    ClaudeEvidenceScenario::Thinking,
    ClaudeEvidenceScenario::PermissionPrompt,
    ClaudeEvidenceScenario::PermissionPersistent,
    ClaudeEvidenceScenario::PlanMode,
    ClaudeEvidenceScenario::Subagent,
    ClaudeEvidenceScenario::Compaction,
    ClaudeEvidenceScenario::McpTool,
    ClaudeEvidenceScenario::Resume,
    ClaudeEvidenceScenario::Interrupt,
    ClaudeEvidenceScenario::Steer,
    ClaudeEvidenceScenario::UsageCost,
    ClaudeEvidenceScenario::MalformedTolerance,
];

/// The public Claude stream-json transport required for every proof.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeEvidenceTransport {
    StreamJson,
}

/// Bounded, redacted metadata for one reviewed Claude scenario artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClaudeScenarioProof {
    pub provider_version: String,
    pub platform: String,
    pub transport: ClaudeEvidenceTransport,
    pub fixture_sha256: String,
    pub attestation_sha256: String,
    pub capture_run_id: String,
    /// Required only for a provider-emitted malformed-output scenario.
    pub malformed_diagnostic_sha256: Option<String>,
}

/// The signer-covered portion of a provider-scoped Claude authority record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct ClaudeAuthorityEvidencePayload {
    pub schema_version: u32,
    pub provider: String,
    pub expires_at_unix_seconds: u64,
    pub compatibility_manifest_sha256: String,
    pub transcript_inventory_sha256: String,
    pub coverage_manifest_sha256: String,
    pub scenarios: BTreeMap<ClaudeEvidenceScenario, ClaudeScenarioProof>,
}

/// A signed, non-effectful Claude authority-evidence record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignedClaudeAuthorityEvidence {
    pub key_id: String,
    pub payload: ClaudeAuthorityEvidencePayload,
    pub signature_hex: String,
}

/// An opaque grant returned only after record and signer validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedClaudeAuthorityEvidence(ClaudeAuthorityEvidencePayload);

impl VerifiedClaudeAuthorityEvidence {
    /// Returns the exact signed compatibility envelope digest approved by this record.
    #[must_use]
    pub fn compatibility_manifest_sha256(&self) -> &str {
        &self.0.compatibility_manifest_sha256
    }

    /// Returns an already shape-checked scenario proof without reading an artifact.
    #[must_use]
    pub fn scenario(&self, scenario: ClaudeEvidenceScenario) -> &ClaudeScenarioProof {
        &self.0.scenarios[&scenario]
    }
}

/// Controlled failure while checking a Claude authority-evidence record.
#[derive(Debug, thiserror::Error)]
pub enum ClaudeAuthorityEvidenceError {
    #[error("Claude authority evidence signer id is invalid")]
    InvalidKeyId,
    #[error("Claude authority evidence signature is malformed")]
    InvalidSignature,
    #[error("Claude authority evidence signature does not verify")]
    SignatureMismatch,
    #[error("Claude authority evidence has expired")]
    Expired,
    #[error("Claude authority evidence schema is unsupported")]
    UnsupportedSchema,
    #[error("Claude authority evidence provider must be exactly claude")]
    ProviderMismatch,
    #[error("Claude authority evidence has an invalid {field} digest")]
    InvalidDigest { field: &'static str },
    #[error("Claude authority evidence scenario set is incomplete or contains an unknown scenario")]
    InvalidScenarioSet,
    #[error("Claude authority evidence proof for {scenario:?} has an invalid {field}")]
    InvalidProof {
        scenario: ClaudeEvidenceScenario,
        field: &'static str,
    },
    #[error(transparent)]
    Trust(#[from] CompatibilityError),
    #[error("Claude authority evidence cannot be encoded")]
    Serialization,
}

impl SignedClaudeAuthorityEvidence {
    /// Verifies signer, signature, expiry, exact provider scope, and bounded proof shape.
    ///
    /// # Errors
    /// Returns an error for every untrusted, expired, incomplete, or malformed record.
    pub fn verify(
        &self,
        keys: &TrustedKeySet,
        now: u64,
    ) -> Result<VerifiedClaudeAuthorityEvidence, ClaudeAuthorityEvidenceError> {
        validate_payload(&self.payload, now)?;
        if !valid_text(&self.key_id, MAX_TEXT_BYTES) {
            return Err(ClaudeAuthorityEvidenceError::InvalidKeyId);
        }
        let key = keys.key(&self.key_id)?;
        let payload = serde_json::to_vec(&self.payload)
            .map_err(|_| ClaudeAuthorityEvidenceError::Serialization)?;
        if self.signature_hex.len() != 128 || !self.signature_hex.bytes().all(lower_hex) {
            return Err(ClaudeAuthorityEvidenceError::InvalidSignature);
        }
        let bytes = hex::decode(&self.signature_hex)
            .map_err(|_| ClaudeAuthorityEvidenceError::InvalidSignature)?;
        let signature = Signature::from_slice(&bytes)
            .map_err(|_| ClaudeAuthorityEvidenceError::InvalidSignature)?;
        key.verify(&payload, &signature)
            .map_err(|_| ClaudeAuthorityEvidenceError::SignatureMismatch)?;
        Ok(VerifiedClaudeAuthorityEvidence(self.payload.clone()))
    }
}

fn validate_payload(
    payload: &ClaudeAuthorityEvidencePayload,
    now: u64,
) -> Result<(), ClaudeAuthorityEvidenceError> {
    if payload.schema_version != SCHEMA_VERSION {
        return Err(ClaudeAuthorityEvidenceError::UnsupportedSchema);
    }
    if payload.provider != "claude" {
        return Err(ClaudeAuthorityEvidenceError::ProviderMismatch);
    }
    if payload.expires_at_unix_seconds < now {
        return Err(ClaudeAuthorityEvidenceError::Expired);
    }
    for (field, digest) in [
        (
            "compatibility_manifest_sha256",
            &payload.compatibility_manifest_sha256,
        ),
        (
            "transcript_inventory_sha256",
            &payload.transcript_inventory_sha256,
        ),
        (
            "coverage_manifest_sha256",
            &payload.coverage_manifest_sha256,
        ),
    ] {
        if !valid_sha256(digest) {
            return Err(ClaudeAuthorityEvidenceError::InvalidDigest { field });
        }
    }
    if payload.scenarios.len() != REQUIRED_SCENARIOS.len()
        || REQUIRED_SCENARIOS
            .iter()
            .any(|scenario| !payload.scenarios.contains_key(scenario))
    {
        return Err(ClaudeAuthorityEvidenceError::InvalidScenarioSet);
    }
    for (scenario, proof) in &payload.scenarios {
        validate_proof(*scenario, proof)?;
    }
    Ok(())
}

fn validate_proof(
    scenario: ClaudeEvidenceScenario,
    proof: &ClaudeScenarioProof,
) -> Result<(), ClaudeAuthorityEvidenceError> {
    for (field, value) in [
        ("provider_version", &proof.provider_version),
        ("capture_run_id", &proof.capture_run_id),
    ] {
        if !valid_text(value, MAX_TEXT_BYTES) {
            return Err(invalid_proof(scenario, field));
        }
    }
    if !valid_platform(&proof.platform) {
        return Err(invalid_proof(scenario, "platform"));
    }
    for (field, digest) in [
        ("fixture_sha256", &proof.fixture_sha256),
        ("attestation_sha256", &proof.attestation_sha256),
    ] {
        if !valid_sha256(digest) {
            return Err(invalid_proof(scenario, field));
        }
    }
    match (scenario, proof.malformed_diagnostic_sha256.as_deref()) {
        (ClaudeEvidenceScenario::MalformedTolerance, Some(digest)) if valid_sha256(digest) => {
            Ok(())
        }
        (ClaudeEvidenceScenario::MalformedTolerance, _) => {
            Err(invalid_proof(scenario, "malformed_diagnostic_sha256"))
        }
        (_, None) => Ok(()),
        _ => Err(invalid_proof(scenario, "malformed_diagnostic_sha256")),
    }
}

const fn invalid_proof(
    scenario: ClaudeEvidenceScenario,
    field: &'static str,
) -> ClaudeAuthorityEvidenceError {
    ClaudeAuthorityEvidenceError::InvalidProof { scenario, field }
}
fn lower_hex(byte: u8) -> bool {
    byte.is_ascii_digit() || matches!(byte, b'a'..=b'f')
}
fn valid_sha256(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(lower_hex)
}
fn valid_text(value: &str, maximum: usize) -> bool {
    !value.trim().is_empty() && value.len() <= maximum && !value.chars().any(char::is_control)
}
fn valid_platform(value: &str) -> bool {
    valid_text(value, MAX_PLATFORM_BYTES)
        && ["macos-", "linux-", "windows-"]
            .into_iter()
            .any(|prefix| value.starts_with(prefix) && value.len() > prefix.len())
}

#[cfg(test)]
#[path = "claude_authority_evidence_tests.rs"]
mod tests;
