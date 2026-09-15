use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};
use gent_runtime::{CachedRuntimeRelease, RuntimeReleaseTrust};
use gent_types::{
    RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseManifest, RuntimeUpdateCheckRequest, RuntimeUpdateCheckState,
    RuntimeUpdateFailure, SignedRuntimeRelease,
};

use super::{metadata_unavailable, packaged_update_checks};
use crate::runtime_update_config::{package_version, platform_target};

fn install_release_material(release: &std::path::Path) {
    let key = SigningKey::from_bytes(&[9; 32]);
    let payload = RuntimeReleaseManifest {
        manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
        release_version: package_version(),
        protocol_min: 1,
        protocol_max: gent_types::PROTOCOL_MAX,
        schema_min: 1,
        schema_max: gent_store::FRESH_SCHEMA_COMPATIBILITY_VERSION,
        minimum_app_version: package_version(),
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: 100,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target: platform_target().unwrap(),
            archive_name: "gent.tar.gz".into(),
            digest_sha256: "b".repeat(64),
            size_bytes: 1,
        },
    };
    let signed = SignedRuntimeRelease {
        key_id: "release-1".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    };
    let trust =
        RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]));
    std::fs::create_dir_all(release).unwrap();
    CachedRuntimeRelease::verify(signed, &trust, 1)
        .unwrap()
        .store(&release.join("runtime-release-cache.json"), &trust, 1)
        .unwrap();
    std::fs::write(
        release.join("runtime-release-trust.json"),
        format!(
            r#"{{"schemaVersion":1,"keys":[{{"keyId":"release-1","publicKeyHex":"{}"}}]}}"#,
            hex::encode(key.verifying_key().to_bytes())
        ),
    )
    .unwrap();
}

#[test]
fn an_installed_release_checks_updates_from_its_packaged_signed_metadata() {
    let root = tempfile::tempdir().unwrap();
    let release = root.path().join("releases/v0.1.0-target");
    install_release_material(&release);
    let checks = packaged_update_checks(&release.join("gentd"), 1).unwrap();
    let report = checks.check(
        RuntimeUpdateCheckRequest {
            channel: RuntimeReleaseChannel::Stable,
        },
        1,
    );
    assert_eq!(report.state, RuntimeUpdateCheckState::Current);
    assert_eq!(report.failure, None);
}

#[test]
fn a_build_without_packaged_metadata_reports_it_unavailable() {
    let root = tempfile::tempdir().unwrap();
    assert!(packaged_update_checks(&root.path().join("gentd"), 1).is_none());
    let report = metadata_unavailable(RuntimeReleaseChannel::Beta);
    assert_eq!(report.state, RuntimeUpdateCheckState::Unavailable);
    assert_eq!(
        report.failure,
        Some(RuntimeUpdateFailure::ReleaseMetadataUnavailable)
    );
}
