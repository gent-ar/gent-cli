use std::{fs, path::PathBuf};

use ed25519_dalek::{Signer, SigningKey};
use gent_types::{
    RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseManifest, SignedRuntimeRelease,
};
use sha2::{Digest, Sha256};

use super::{RuntimeUpdateBootstrapConfig, verify_if_enabled};
use crate::runtime_update_config::{package_version, platform_target};

struct Fixture {
    cache: PathBuf,
    trust: PathBuf,
    release: PathBuf,
    archive: PathBuf,
    archive_manifest: PathBuf,
}

impl Fixture {
    fn config(&self) -> RuntimeUpdateBootstrapConfig<'_> {
        RuntimeUpdateBootstrapConfig {
            enabled: true,
            cache_path: Some(&self.cache),
            trust_path: Some(&self.trust),
            release_path: Some(&self.release),
            archive_path: Some(&self.archive),
            archive_manifest_path: Some(&self.archive_manifest),
            now_unix_seconds: 1,
        }
    }
}

fn fixture(root: &std::path::Path) -> Fixture {
    let key = SigningKey::from_bytes(&[3; 32]);
    let target = platform_target().unwrap();
    let version = package_version();
    let archive = root.join(format!(
        "gent-v{}.{}.{}-target.tar.gz",
        version.major, version.minor, version.patch
    ));
    fs::write(&archive, b"verified archive bytes").unwrap();
    let digest = hex::encode(Sha256::digest(fs::read(&archive).unwrap()));
    let archive_name = archive.file_name().unwrap().to_str().unwrap().to_owned();
    let archive_manifest = root.join("archive.json");
    fs::write(
        &archive_manifest,
        serde_json::json!({
            "schemaVersion": 1,
            "version": format!("v{}.{}.{}", version.major, version.minor, version.patch),
            "target": target,
            "archive": {"name": archive_name, "sha256": digest, "size": 22},
            "binaries": ["gent", "gentd"],
        })
        .to_string(),
    )
    .unwrap();
    let payload = RuntimeReleaseManifest {
        manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
        release_version: version,
        protocol_min: 1,
        protocol_max: gent_types::PROTOCOL_MAX,
        schema_min: 1,
        schema_max: gent_store::FRESH_SCHEMA_COMPATIBILITY_VERSION,
        minimum_app_version: version,
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: 10,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target,
            archive_name,
            digest_sha256: digest,
            size_bytes: 22,
        },
    };
    let release = SignedRuntimeRelease {
        key_id: "release-1".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    };
    let release_path = root.join("release.json");
    fs::write(&release_path, serde_json::to_vec(&release).unwrap()).unwrap();
    let trust = root.join("trust.json");
    fs::write(
        &trust,
        serde_json::json!({"schemaVersion": 1, "keys": [{
            "keyId": "release-1", "publicKeyHex": hex::encode(key.verifying_key().to_bytes()),
        }]})
        .to_string(),
    )
    .unwrap();
    let cache = root.join("cache.json");
    Fixture {
        cache,
        trust,
        release: release_path,
        archive,
        archive_manifest,
    }
}

#[test]
fn verified_material_creates_a_revalidatable_cache() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = fixture(directory.path());
    let config = fixture.config();
    assert!(verify_if_enabled(config).unwrap());
    assert!(config.cache_path.unwrap().is_file());
}

#[test]
fn mismatched_archive_preserves_a_missing_cache() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = fixture(directory.path());
    let config = fixture.config();
    fs::write(config.archive_path.unwrap(), b"tampered").unwrap();
    assert!(verify_if_enabled(config).is_err());
    assert!(!config.cache_path.unwrap().exists());
}

#[test]
fn strict_trust_document_rejects_unknown_fields() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = fixture(directory.path());
    let config = fixture.config();
    let trust = config.trust_path.unwrap();
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(trust).unwrap()).unwrap();
    value["unexpected"] = serde_json::Value::Bool(true);
    fs::write(trust, value.to_string()).unwrap();
    assert!(verify_if_enabled(config).is_err());
    assert!(!config.cache_path.unwrap().exists());
}
