//! Content-free contracts for signed Gent runtime release updates.

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
    HealthCheckFailed,
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

/// One append-only durable checkpoint in a runtime-update attempt.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeUpdateRecord {
    pub attempt_id: String,
    pub revision: u64,
    pub artifact_digest_sha256: String,
    pub status: RuntimeUpdateStatus,
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
mod tests {
    use serde_json::json;

    use super::{RuntimeUpdateRecord, RuntimeUpdateStage, RuntimeUpdateStatus, RuntimeVersion};

    #[test]
    fn status_uses_a_stable_content_free_camel_case_contract() {
        let status = RuntimeUpdateStatus {
            stage: RuntimeUpdateStage::ReadyToActivate,
            release_version: Some(RuntimeVersion {
                major: 1,
                minor: 2,
                patch: 3,
            }),
            forward_only_schema: true,
            failure: None,
        };
        assert_eq!(
            serde_json::to_value(status).unwrap(),
            json!({
                "stage": "readyToActivate",
                "releaseVersion": { "major": 1, "minor": 2, "patch": 3 },
                "forwardOnlySchema": true,
                "failure": null,
            })
        );
    }

    #[test]
    fn update_record_keeps_attempt_order_separate_from_release_state() {
        let record = RuntimeUpdateRecord {
            attempt_id: "attempt-1".into(),
            revision: 2,
            artifact_digest_sha256: "a".repeat(64),
            status: RuntimeUpdateStatus::default(),
        };
        assert_eq!(serde_json::to_value(record).unwrap()["revision"], 2);
    }
}
