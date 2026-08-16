use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};
use gent_core::{RuntimeUpdateEvent, reduce_runtime_update};
use gent_ports::{IngressMode, Ledger, runtime_update::RuntimeUpdateJournal};
use gent_runtime::{CachedRuntimeRelease, RuntimeReleaseTrust};
use gent_store::SqliteLedger;
use gent_types::{
    HostEpoch, RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseIdentity, RuntimeReleaseManifest, RuntimeStagingReceipt, RuntimeUpdateHandoff,
    RuntimeUpdateRecord, RuntimeUpdateStage, RuntimeUpdateStatus, RuntimeVersion,
    SignedRuntimeRelease,
};

use super::{RuntimeUpdateRecoverConfig, confirm_if_enabled, open_confirmed};
use crate::runtime_update_config::platform_target;

fn version() -> RuntimeVersion {
    RuntimeVersion {
        major: 0,
        minor: 1,
        patch: 4,
    }
}

fn prepared(directory: &std::path::Path) -> (String, std::path::PathBuf) {
    let key = SigningKey::from_bytes(&[5; 32]);
    let artifact = RuntimeReleaseArtifact {
        target: platform_target().unwrap(),
        archive_name: "gent.tar.gz".into(),
        digest_sha256: "a".repeat(64),
        size_bytes: 1,
    };
    let release = SignedRuntimeRelease {
        key_id: "release-1".into(),
        signature_hex: hex::encode(
            key.sign(
                &serde_json::to_vec(&RuntimeReleaseManifest {
                    manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
                    release_version: version(),
                    protocol_min: 1,
                    protocol_max: gent_types::PROTOCOL_MAX,
                    schema_min: 1,
                    schema_max: gent_store::CURRENT_SCHEMA_VERSION,
                    minimum_app_version: version(),
                    channel: RuntimeReleaseChannel::Stable,
                    rollout_percent: 100,
                    expires_at_unix_seconds: 100,
                    revoked: false,
                    forward_only_schema: false,
                    artifact: artifact.clone(),
                })
                .unwrap(),
            )
            .to_bytes(),
        ),
        payload: RuntimeReleaseManifest {
            manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
            release_version: version(),
            protocol_min: 1,
            protocol_max: gent_types::PROTOCOL_MAX,
            schema_min: 1,
            schema_max: gent_store::CURRENT_SCHEMA_VERSION,
            minimum_app_version: version(),
            channel: RuntimeReleaseChannel::Stable,
            rollout_percent: 100,
            expires_at_unix_seconds: 100,
            revoked: false,
            forward_only_schema: false,
            artifact,
        },
    };
    let trust =
        RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]));
    let cache = directory.join("release.json");
    CachedRuntimeRelease::verify(release, &trust, 1)
        .unwrap()
        .store(&cache, &trust, 1)
        .unwrap();
    (
        format!("release-1:{}", hex::encode(key.verifying_key().to_bytes())),
        cache,
    )
}

fn record() -> RuntimeUpdateRecord {
    let digest = "a".repeat(64);
    RuntimeUpdateRecord {
        attempt_id: "handoff-1".into(),
        revision: 1,
        artifact_digest_sha256: digest.clone(),
        status: reduce_runtime_update(
            RuntimeUpdateStatus {
                stage: RuntimeUpdateStage::ReadyToActivate,
                release_version: Some(version()),
                forward_only_schema: false,
                failure: None,
            },
            RuntimeUpdateEvent::HandoffRequested {
                ingress_closed: true,
            },
            None,
        )
        .status,
        handoff: RuntimeUpdateHandoff {
            origin_host_epoch: Some(HostEpoch(1)),
            release: Some(RuntimeReleaseIdentity {
                key_id: "release-1".into(),
                release_version: version(),
                target: platform_target().unwrap(),
                artifact_digest_sha256: digest.clone(),
            }),
            staging_receipt: Some(RuntimeStagingReceipt {
                attempt_id: "handoff-1".into(),
                artifact_digest_sha256: digest,
            }),
        },
    }
}

#[test]
fn disabled_recovery_does_not_create_a_ledger() {
    let directory = tempfile::tempdir().unwrap();
    assert!(
        confirm_if_enabled(
            directory.path(),
            RuntimeUpdateRecoverConfig {
                enabled: false,
                attempt_id: None,
                cache_path: None,
                trust_path: None,
                keys: &[],
                now_unix_seconds: 1,
            }
        )
        .unwrap()
        .is_none()
    );
    assert!(!directory.path().join("gent.db").exists());
}

#[test]
fn recovery_confirms_exact_handoff_then_fences_the_new_epoch() {
    let directory = tempfile::tempdir().unwrap();
    let (key, cache) = prepared(directory.path());
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    ledger.save_runtime_update(&record()).unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    let recovery = confirm_if_enabled(
        directory.path(),
        RuntimeUpdateRecoverConfig {
            enabled: true,
            attempt_id: Some("handoff-1"),
            cache_path: Some(&cache),
            trust_path: None,
            keys: &[key],
            now_unix_seconds: 1,
        },
    )
    .unwrap()
    .unwrap();
    assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Closed);
    assert_eq!(
        open_confirmed(directory.path(), &recovery).unwrap(),
        HostEpoch(2)
    );
    assert_eq!(
        ledger.host_ingress().unwrap(),
        gent_ports::HostIngress {
            epoch: HostEpoch(2),
            mode: IngressMode::Open
        }
    );
    assert_eq!(
        ledger
            .find_runtime_update("handoff-1")
            .unwrap()
            .unwrap()
            .status
            .stage,
        RuntimeUpdateStage::Activated
    );
}

#[test]
fn recovery_mismatch_leaves_the_old_epoch_closed() {
    let directory = tempfile::tempdir().unwrap();
    let (key, cache) = prepared(directory.path());
    let ledger = SqliteLedger::open(directory.path().join("gent.db")).unwrap();
    let mut incorrect = record();
    incorrect.handoff.release.as_mut().unwrap().key_id = "other".into();
    ledger.save_runtime_update(&incorrect).unwrap();
    ledger.close_ingress(HostEpoch(1)).unwrap();
    assert!(
        confirm_if_enabled(
            directory.path(),
            RuntimeUpdateRecoverConfig {
                enabled: true,
                attempt_id: Some("handoff-1"),
                cache_path: Some(&cache),
                trust_path: None,
                keys: &[key],
                now_unix_seconds: 1,
            }
        )
        .is_err()
    );
    assert_eq!(ledger.host_ingress().unwrap().mode, IngressMode::Closed);
    assert_eq!(
        ledger.find_runtime_update("handoff-1").unwrap(),
        Some(incorrect)
    );
}
