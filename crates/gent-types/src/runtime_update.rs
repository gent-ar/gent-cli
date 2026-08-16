//! Content-free contracts for signed Gent runtime release updates.

use crate::HostEpoch;
use serde::{Deserialize, Serialize};

/// The only runtime release-manifest contract understood by this build.
pub const RUNTIME_RELEASE_MANIFEST_VERSION: u16 = 1;

/// A three-part Gent runtime or app version.
#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

/// Explicit release channel selection; stable remains the default caller policy.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeReleaseChannel {
    Stable,
    Beta,
    Canary,
}

/// A target-specific archive allowed by a signed runtime release manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReleaseArtifact {
    pub target: String,
    pub archive_name: String,
    pub digest_sha256: String,
    pub size_bytes: u64,
}

/// Canonical, signed metadata required before a runtime artifact may be staged.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReleaseManifest {
    pub manifest_version: u16,
    pub release_version: RuntimeVersion,
    pub protocol_min: u16,
    pub protocol_max: u16,
    pub schema_min: u32,
    pub schema_max: u32,
    pub minimum_app_version: RuntimeVersion,
    pub channel: RuntimeReleaseChannel,
    pub rollout_percent: u8,
    pub expires_at_unix_seconds: u64,
    pub revoked: bool,
    pub forward_only_schema: bool,
    pub artifact: RuntimeReleaseArtifact,
}

/// Signed envelope for a runtime release manifest.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignedRuntimeRelease {
    pub key_id: String,
    pub payload: RuntimeReleaseManifest,
    pub signature_hex: String,
}

/// Durable updater phase; side effects are owned by separate ports/bootstrapper code.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeUpdateStage {
    Idle,
    Available,
    Staged,
    HealthChecking,
    ReadyToActivate,
    HandoffRequested,
    Activated,
    RolledBack,
    ReadOnlyUpdateRequired,
    Failed,
}

/// A specific safety condition that prevents staging, activation, or rollback.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeUpdateFailure {
    SignatureInvalid,
    Expired,
    Revoked,
    InvalidArtifactDigest,
    IncompatibleProtocol,
    IncompatibleSchema,
    IncompatibleApp,
    /// This build has no configured, independently trusted release metadata source.
    ReleaseMetadataUnavailable,
    StagingFailed,
    HealthCheckFailed,
    ActivationFailed,
    ForwardOnlyRollback,
    IngressNotClosed,
}

/// Content-free durable state for a single runtime update attempt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateStatus {
    pub stage: RuntimeUpdateStage,
    pub release_version: Option<RuntimeVersion>,
    pub forward_only_schema: bool,
    pub failure: Option<RuntimeUpdateFailure>,
}

/// Opaque receipt for a digest-verified staged runtime archive.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStagingReceipt {
    pub attempt_id: String,
    pub artifact_digest_sha256: String,
}

/// Immutable release facts a successor must match before it may confirm a handoff.
///
/// This is deliberately content-free: it identifies the already verified signed release without
/// retaining download locations, credentials, or an activation command.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeReleaseIdentity {
    pub key_id: String,
    pub release_version: RuntimeVersion,
    pub target: String,
    pub artifact_digest_sha256: String,
}

/// Durable facts which bind an update to the old host and its exact staged release.
///
/// Historic journal rows decode with no handoff facts. They remain readable but can never be
/// treated as successor-confirmable, which avoids retroactively inventing a host fence.
#[derive(Clone, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateHandoff {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin_host_epoch: Option<HostEpoch>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<RuntimeReleaseIdentity>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub staging_receipt: Option<RuntimeStagingReceipt>,
}

/// One append-only durable checkpoint in a runtime-update attempt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateRecord {
    pub attempt_id: String,
    pub revision: u64,
    pub artifact_digest_sha256: String,
    pub status: RuntimeUpdateStatus,
    #[serde(default)]
    pub handoff: RuntimeUpdateHandoff,
}

/// A user-selected channel for a read-only runtime update check.
///
/// This request cannot authorize download, staging, activation, or a binary
/// replacement. Those operations intentionally have no public DTO yet.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateCheckRequest {
    pub channel: RuntimeReleaseChannel,
}

/// The content-free outcome of a read-only runtime update check.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeUpdateCheckState {
    Current,
    Available,
    ReadOnlyUpdateRequired,
    Unavailable,
}

/// A verified candidate reported by a read-only check.
///
/// The digest identifies the exact proposed release without exposing a source
/// URL, credentials, or an activation mechanism.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateCandidate {
    pub release_version: RuntimeVersion,
    pub artifact_digest_sha256: String,
    pub forward_only_schema: bool,
}

/// A read-only report describing the running runtime and any verified offer.
///
/// `candidate` is present only for [`RuntimeUpdateCheckState::Available`].
/// `failure` is present only when the check requires a read-only state or
/// cannot produce a trustworthy answer.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateCheckReport {
    pub current_version: RuntimeVersion,
    pub channel: RuntimeReleaseChannel,
    pub state: RuntimeUpdateCheckState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub candidate: Option<RuntimeUpdateCandidate>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure: Option<RuntimeUpdateFailure>,
}

impl Default for RuntimeUpdateStatus {
    fn default() -> Self {
        Self {
            stage: RuntimeUpdateStage::Idle,
            release_version: None,
            forward_only_schema: false,
            failure: None,
        }
    }
}

#[cfg(test)]
#[path = "runtime_update_tests.rs"]
mod tests;
