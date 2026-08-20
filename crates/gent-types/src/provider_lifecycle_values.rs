//! Durable provider executable and normalized event values.

use serde::{Deserialize, Serialize};

use crate::{RootActivity, WorkPhase};

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunVersionLock {
    pub provider: String,
    pub canonical_path: String,
    pub file_identity: String,
    pub digest_sha256: String,
    pub version: String,
    pub compatibility_entry: String,
}

/// Immutable public-provider executable identity verified after a Gent-owned install.
///
/// This value is independent from a run lock: the ledger records it as installation
/// provenance, while every run still receives its own immutable `RunVersionLock`.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedProviderLock {
    pub run_lock: RunVersionLock,
}

/// Exact signed-policy selection and app runtime identity used for one installation.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderInstallProvenance {
    pub package_name: String,
    pub package_version: String,
    pub package_integrity: String,
    pub package_policy_digest_sha256: String,
    pub node_runtime_digest_sha256: String,
    /// Digest of the exact signed ordinary-authority release reauthorized immediately before
    /// this installation's npm effect.
    pub release_artifact_digest_sha256: String,
    /// Fingerprint of the exact accepted command that produced this installation.
    pub receipt_fingerprint_sha256: String,
}

/// A verified provider executable together with immutable installation provenance.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionedProviderInstallation {
    pub lock: ProvisionedProviderLock,
    pub provenance: ProviderInstallProvenance,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum NormalizedProviderEvent {
    Output {
        text: String,
        is_partial: bool,
    },
    /// Provider-supplied reasoning content. This remains distinct from assistant output so a
    /// client can apply its own disclosure policy without treating it as an answer.
    Thinking {
        text: String,
        is_partial: bool,
    },
    TurnStarted {
        turn_id: String,
    },
    TurnEnded {
        turn_id: String,
    },
    RootActivity {
        activity: RootActivity,
    },
    ChildStarted {
        child_id: String,
        parent_tool_use_id: String,
    },
    ChildTerminal {
        child_id: String,
        phase: WorkPhase,
    },
    CommandTerminal {
        command_id: String,
        phase: WorkPhase,
    },
    DecisionSettled {
        decision_id: String,
    },
    TransportDiagnostic {
        classification: String,
    },
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ProviderEvent {
    Output { text: String },
    DecisionSettled { decision_id: String },
    Terminal { reason: String },
}
