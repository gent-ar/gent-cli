use serde_json::json;

use super::{
    RuntimeReleaseChannel, RuntimeUpdateCandidate, RuntimeUpdateCheckReport,
    RuntimeUpdateCheckState, RuntimeUpdateFailure, RuntimeUpdateStage, RuntimeUpdateStatus,
    RuntimeVersion,
};

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
fn check_report_is_content_free_and_does_not_imply_activation() {
    let report = RuntimeUpdateCheckReport {
        current_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        channel: RuntimeReleaseChannel::Stable,
        state: RuntimeUpdateCheckState::Available,
        candidate: Some(RuntimeUpdateCandidate {
            release_version: RuntimeVersion {
                major: 1,
                minor: 1,
                patch: 0,
            },
            artifact_digest_sha256: "a".repeat(64),
            forward_only_schema: false,
        }),
        failure: None,
    };
    let encoded = serde_json::to_value(report).unwrap();
    assert_eq!(encoded["state"], "available");
    assert!(encoded.get("failure").is_none());
    assert!(!encoded.to_string().contains("activate"));
    assert!(!encoded.to_string().contains("archiveName"));
}

#[test]
fn unavailable_check_distinguishes_missing_metadata_from_bad_signatures() {
    let report = RuntimeUpdateCheckReport {
        current_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        channel: RuntimeReleaseChannel::Stable,
        state: RuntimeUpdateCheckState::Unavailable,
        candidate: None,
        failure: Some(RuntimeUpdateFailure::ReleaseMetadataUnavailable),
    };
    assert_eq!(
        serde_json::to_value(report).unwrap()["failure"],
        "releaseMetadataUnavailable"
    );
}
