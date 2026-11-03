use gent_types::HostEpoch;
use serde::{Deserialize, Serialize};

pub const RUNTIME_OWNER_CONTROL_CAPABILITY: &str = "runtime-owner-control-v1";
pub const RUNTIME_OWNER_QUIESCE_REJECTION_CAPABILITY: &str = "runtime-owner-quiesce-rejection-v1";
pub const RUNTIME_OWNER_IDENTITY_CAPABILITY: &str = "runtime-owner-identity-v1";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeOwnerQuiesceEffect {
    NotApplied,
    MayHaveChanged,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum RuntimeOwnerQuiesceReason {
    Busy,
    AdmissionUnavailable,
    IdentityUnavailable,
    IdentityMismatch,
    LedgerOrCommitUnavailable,
}
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeOwnerQuiesceFailure {
    pub effect: RuntimeOwnerQuiesceEffect,
    pub reason: RuntimeOwnerQuiesceReason,
    pub message: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RuntimeOwnerFrame {
    RuntimeOwnerStatus {
        request_id: String,
    },
    RuntimeOwnerIdentity {
        request_id: String,
    },
    RuntimeOwnerIdentityState {
        request_id: String,
        host_epoch: HostEpoch,
        process_id: u32,
        executable_digest_sha256: String,
        canonical_executable_path: std::path::PathBuf,
        canonical_data_directory: std::path::PathBuf,
        boot_nonce: String,
    },
    RuntimeOwnerIdentityUnavailable {
        request_id: String,
    },
    QuiesceRuntimeOwner {
        request_id: String,
        expected_host_epoch: HostEpoch,
        expected_process_id: u32,
        expected_executable_digest_sha256: String,
    },
    RuntimeOwnerState {
        request_id: String,
        host_epoch: HostEpoch,
        process_id: u32,
        executable_digest_sha256: String,
        quiesced: bool,
    },
    RuntimeOwnerQuiesced {
        request_id: String,
        host_epoch: HostEpoch,
        process_id: u32,
        executable_digest_sha256: String,
    },
    RuntimeOwnerQuiesceRejected {
        request_id: String,
        failure: RuntimeOwnerQuiesceFailure,
    },
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn owner_quiesce_wire_requires_the_complete_exact_instance_tuple() {
        let raw = json!({
            "type":"quiesceRuntimeOwner", "requestId":"repair", "expectedHostEpoch":7,
            "expectedProcessId":321, "expectedExecutableDigestSha256":"a".repeat(64),
        });
        let frame: RuntimeOwnerFrame = serde_json::from_value(raw.clone()).unwrap();
        assert_eq!(serde_json::to_value(frame).unwrap(), raw);
        for field in [
            "expectedHostEpoch",
            "expectedProcessId",
            "expectedExecutableDigestSha256",
        ] {
            let mut missing = raw.clone();
            missing.as_object_mut().unwrap().remove(field);
            assert!(serde_json::from_value::<RuntimeOwnerFrame>(missing).is_err());
        }
    }

    #[test]
    fn owner_state_and_ack_preserve_identity_without_granting_restart() {
        for frame in [
            RuntimeOwnerFrame::RuntimeOwnerState {
                request_id: "status".into(),
                host_epoch: HostEpoch(7),
                process_id: 321,
                executable_digest_sha256: "a".repeat(64),
                quiesced: false,
            },
            RuntimeOwnerFrame::RuntimeOwnerQuiesced {
                request_id: "repair".into(),
                host_epoch: HostEpoch(7),
                process_id: 321,
                executable_digest_sha256: "a".repeat(64),
            },
        ] {
            let value = serde_json::to_value(&frame).unwrap();
            assert_eq!(
                serde_json::from_value::<RuntimeOwnerFrame>(value.clone()).unwrap(),
                frame
            );
            assert!(value.get("restart").is_none());
        }
    }
}
