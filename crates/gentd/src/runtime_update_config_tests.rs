use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};
use gent_runtime::{CachedRuntimeRelease, RuntimeReleaseTrust};
use gent_types::{
    RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseManifest, RuntimeUpdateCheckRequest, RuntimeUpdateCheckState, RuntimeVersion,
    SignedRuntimeRelease,
};

use super::{load, load_keys, parse_keys, platform_target};

#[test]
fn key_parser_fails_closed() {
    assert!(parse_keys(&["key:00".into()]).is_err());
    assert!(parse_keys(&[format!("key:{}", "A".repeat(64))]).is_err());
}

#[test]
fn target_is_one_of_the_published_release_targets() {
    assert!(platform_target().is_ok());
}

#[test]
fn enabled_check_requires_a_trusted_cache_and_revalidates_it() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let payload = RuntimeReleaseManifest {
        manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
        release_version: RuntimeVersion {
            major: 9,
            minor: 0,
            patch: 0,
        },
        protocol_min: 1,
        protocol_max: gent_types::PROTOCOL_MAX,
        schema_min: 1,
        schema_max: gent_store::CURRENT_SCHEMA_VERSION,
        minimum_app_version: RuntimeVersion {
            major: 0,
            minor: 1,
            patch: 0,
        },
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: 10,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target: platform_target().unwrap(),
            archive_name: "gent.tar.gz".into(),
            digest_sha256: "a".repeat(64),
            size_bytes: 1,
        },
    };
    let release = SignedRuntimeRelease {
        key_id: "release-1".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    };
    let trust =
        RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]));
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("runtime-release.json");
    CachedRuntimeRelease::verify(release, &trust, 1)
        .unwrap()
        .store(&path, &trust, 1)
        .unwrap();
    let key_text = format!("release-1:{}", hex::encode(key.verifying_key().to_bytes()));
    let checks = load(true, Some(&path), None, &[key_text], 1)
        .unwrap()
        .unwrap();
    assert_eq!(
        checks
            .check(
                RuntimeUpdateCheckRequest {
                    channel: RuntimeReleaseChannel::Stable
                },
                1
            )
            .state,
        RuntimeUpdateCheckState::Available
    );
    assert!(load(true, Some(&path), None, &[], 1).is_err());
}

#[test]
fn trust_document_is_strict_and_can_supply_the_only_key() {
    let key = SigningKey::from_bytes(&[8; 32]);
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("trust.json");
    std::fs::write(
        &path,
        format!(
            r#"{{"schemaVersion":1,"keys":[{{"keyId":"release-1","publicKeyHex":"{}"}}]}}"#,
            hex::encode(key.verifying_key().to_bytes())
        ),
    )
    .unwrap();
    assert!(load_keys(Some(&path), &[]).is_ok());
    std::fs::write(&path, r#"{"schemaVersion":2,"keys":[]}"#).unwrap();
    assert!(load_keys(Some(&path), &[]).is_err());
}
