//! Signed, provider-scoped Codex evidence records for a future authority composition.
//!
//! This module only verifies supplied, signed metadata. It never reads transcript files, starts a
//! provider, or treats a successful verification as proof that an external provider was used.

use std::collections::BTreeMap;

use ed25519_dalek::{Signature, Verifier};
use serde::{Deserialize, Serialize};

use crate::compatibility::{CompatibilityError, TrustedKeySet};

const SCHEMA_VERSION: u32 = 1;
const MAX_TEXT_BYTES: usize = 128;
const MAX_PLATFORM_BYTES: usize = 64;

/// The fixed public Codex scenario set required before a record can be considered complete.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexEvidenceScenario {
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

const REQUIRED_SCENARIOS: [CodexEvidenceScenario; 15] = [
    CodexEvidenceScenario::FullTurn,
    CodexEvidenceScenario::ToolUse,
    CodexEvidenceScenario::ToolError,
    CodexEvidenceScenario::Thinking,
    CodexEvidenceScenario::PermissionPrompt,
    CodexEvidenceScenario::PermissionPersistent,
    CodexEvidenceScenario::PlanMode,
    CodexEvidenceScenario::Subagent,
    CodexEvidenceScenario::Compaction,
    CodexEvidenceScenario::McpTool,
    CodexEvidenceScenario::Resume,
    CodexEvidenceScenario::Interrupt,
    CodexEvidenceScenario::Steer,
    CodexEvidenceScenario::UsageCost,
    CodexEvidenceScenario::MalformedTolerance,
];

/// The documented Codex app-server transport bound by every proof in this record.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum CodexEvidenceTransport {
    JsonRpc,
}

/// Bounded, redacted metadata for one reviewed scenario artifact.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodexScenarioProof {
    pub provider_version: String,
    pub platform: String,
    pub transport: CodexEvidenceTransport,
    pub fixture_sha256: String,
    pub attestation_sha256: String,
    pub capture_run_id: String,
    /// Required only for the documented malformed-output scenario.
    pub malformed_diagnostic_sha256: Option<String>,
}

/// The signer-covered portion of a provider-scoped authority record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct CodexAuthorityEvidencePayload {
    pub schema_version: u32,
    pub provider: String,
    pub expires_at_unix_seconds: u64,
    /// Digest of the whole signed compatibility envelope, not a lossy entry projection.
    pub compatibility_manifest_sha256: String,
    pub transcript_inventory_sha256: String,
    pub coverage_manifest_sha256: String,
    pub scenarios: BTreeMap<CodexEvidenceScenario, CodexScenarioProof>,
}

/// A signed, non-effectful Codex authority-evidence record.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct SignedCodexAuthorityEvidence {
    pub key_id: String,
    pub payload: CodexAuthorityEvidencePayload,
    pub signature_hex: String,
}

/// An opaque grant returned only after record and signer validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerifiedCodexAuthorityEvidence(CodexAuthorityEvidencePayload);

impl VerifiedCodexAuthorityEvidence {
    /// Returns the exact compatibility envelope digest this evidence record was approved against.
    #[must_use]
    pub fn compatibility_manifest_sha256(&self) -> &str {
        &self.0.compatibility_manifest_sha256
    }

    /// Returns the signer-covered scenario proof without reading its referenced artifact.
    #[must_use]
    pub fn scenario(&self, scenario: CodexEvidenceScenario) -> &CodexScenarioProof {
        // `verify` rejects incomplete maps before constructing this opaque value.
        &self.0.scenarios[&scenario]
    }
}

/// Controlled failure while checking an authority-evidence record.
#[derive(Debug, thiserror::Error)]
pub enum CodexAuthorityEvidenceError {
    #[error("Codex authority evidence signer id is invalid")]
    InvalidKeyId,
    #[error("Codex authority evidence signature is malformed")]
    InvalidSignature,
    #[error("Codex authority evidence signature does not verify")]
    SignatureMismatch,
    #[error("Codex authority evidence has expired")]
    Expired,
    #[error("Codex authority evidence schema is unsupported")]
    UnsupportedSchema,
    #[error("Codex authority evidence provider must be exactly codex")]
    ProviderMismatch,
    #[error("Codex authority evidence has an invalid {field} digest")]
    InvalidDigest { field: &'static str },
    #[error("Codex authority evidence scenario set is incomplete or contains an unknown scenario")]
    InvalidScenarioSet,
    #[error("Codex authority evidence proof for {scenario:?} has an invalid {field}")]
    InvalidProof {
        scenario: CodexEvidenceScenario,
        field: &'static str,
    },
    #[error(transparent)]
    Trust(#[from] CompatibilityError),
    #[error("Codex authority evidence cannot be encoded")]
    Serialization,
}

impl SignedCodexAuthorityEvidence {
    /// Verifies signer trust, signature, expiry, fixed Codex scope, and bounded record shape.
    ///
    /// This has no filesystem, process, or network effect and does not verify external evidence.
    ///
    /// # Errors
    /// Returns an error when any signed field, trusted key, expiry, or proof shape is invalid.
    pub fn verify(
        &self,
        keys: &TrustedKeySet,
        now_unix_seconds: u64,
    ) -> Result<VerifiedCodexAuthorityEvidence, CodexAuthorityEvidenceError> {
        validate_payload(&self.payload, now_unix_seconds)?;
        if !valid_text(&self.key_id, MAX_TEXT_BYTES) {
            return Err(CodexAuthorityEvidenceError::InvalidKeyId);
        }
        let key = keys.key(&self.key_id)?;
        let payload = serde_json::to_vec(&self.payload)
            .map_err(|_| CodexAuthorityEvidenceError::Serialization)?;
        if self.signature_hex.len() != 128
            || !self
                .signature_hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
        {
            return Err(CodexAuthorityEvidenceError::InvalidSignature);
        }
        let bytes = hex::decode(&self.signature_hex)
            .map_err(|_| CodexAuthorityEvidenceError::InvalidSignature)?;
        let signature = Signature::from_slice(&bytes)
            .map_err(|_| CodexAuthorityEvidenceError::InvalidSignature)?;
        key.verify(&payload, &signature)
            .map_err(|_| CodexAuthorityEvidenceError::SignatureMismatch)?;
        Ok(VerifiedCodexAuthorityEvidence(self.payload.clone()))
    }
}

fn validate_payload(
    payload: &CodexAuthorityEvidencePayload,
    now_unix_seconds: u64,
) -> Result<(), CodexAuthorityEvidenceError> {
    if payload.schema_version != SCHEMA_VERSION {
        return Err(CodexAuthorityEvidenceError::UnsupportedSchema);
    }
    if payload.provider != "codex" {
        return Err(CodexAuthorityEvidenceError::ProviderMismatch);
    }
    if payload.expires_at_unix_seconds < now_unix_seconds {
        return Err(CodexAuthorityEvidenceError::Expired);
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
            return Err(CodexAuthorityEvidenceError::InvalidDigest { field });
        }
    }
    if payload.scenarios.len() != REQUIRED_SCENARIOS.len()
        || REQUIRED_SCENARIOS
            .iter()
            .any(|scenario| !payload.scenarios.contains_key(scenario))
    {
        return Err(CodexAuthorityEvidenceError::InvalidScenarioSet);
    }
    for (scenario, proof) in &payload.scenarios {
        validate_proof(*scenario, proof)?;
    }
    Ok(())
}

fn validate_proof(
    scenario: CodexEvidenceScenario,
    proof: &CodexScenarioProof,
) -> Result<(), CodexAuthorityEvidenceError> {
    if !valid_text(&proof.provider_version, MAX_TEXT_BYTES) {
        return Err(invalid_proof(scenario, "provider_version"));
    }
    if !valid_platform(&proof.platform) {
        return Err(invalid_proof(scenario, "platform"));
    }
    if !valid_sha256(&proof.fixture_sha256) {
        return Err(invalid_proof(scenario, "fixture_sha256"));
    }
    if !valid_sha256(&proof.attestation_sha256) {
        return Err(invalid_proof(scenario, "attestation_sha256"));
    }
    if !valid_text(&proof.capture_run_id, MAX_TEXT_BYTES) {
        return Err(invalid_proof(scenario, "capture_run_id"));
    }
    match (scenario, proof.malformed_diagnostic_sha256.as_deref()) {
        (CodexEvidenceScenario::MalformedTolerance, Some(digest)) if valid_sha256(digest) => {}
        (CodexEvidenceScenario::MalformedTolerance, _) => {
            return Err(invalid_proof(scenario, "malformed_diagnostic_sha256"));
        }
        (_, None) => {}
        _ => return Err(invalid_proof(scenario, "malformed_diagnostic_sha256")),
    }
    Ok(())
}

const fn invalid_proof(
    scenario: CodexEvidenceScenario,
    field: &'static str,
) -> CodexAuthorityEvidenceError {
    CodexAuthorityEvidenceError::InvalidProof { scenario, field }
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
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
#[path = "codex_authority_evidence_tests.rs"]
mod tests;
