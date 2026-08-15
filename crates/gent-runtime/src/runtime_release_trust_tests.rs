use std::collections::BTreeMap;

use ed25519_dalek::{Signer, SigningKey};
use gent_types::{
    RUNTIME_RELEASE_MANIFEST_VERSION, RuntimeReleaseArtifact, RuntimeReleaseChannel,
    RuntimeReleaseManifest, RuntimeVersion, SignedRuntimeRelease,
};

use super::{RuntimeReleaseTrust, RuntimeReleaseTrustError};

fn release(key: &SigningKey) -> SignedRuntimeRelease {
    let payload = RuntimeReleaseManifest {
        manifest_version: RUNTIME_RELEASE_MANIFEST_VERSION,
        release_version: RuntimeVersion {
            major: 1,
            minor: 2,
            patch: 3,
        },
        protocol_min: 1,
        protocol_max: 2,
        schema_min: 1,
        schema_max: 2,
        minimum_app_version: RuntimeVersion {
            major: 1,
            minor: 0,
            patch: 0,
        },
        channel: RuntimeReleaseChannel::Stable,
        rollout_percent: 100,
        expires_at_unix_seconds: 10,
        revoked: false,
        forward_only_schema: false,
        artifact: RuntimeReleaseArtifact {
            target: "aarch64-apple-darwin".into(),
            archive_name: "gent.tar.gz".into(),
            digest_sha256: "a".repeat(64),
            size_bytes: 42,
        },
    };
    SignedRuntimeRelease {
        key_id: "release-1".into(),
        signature_hex: hex::encode(key.sign(&serde_json::to_vec(&payload).unwrap()).to_bytes()),
        payload,
    }
}

fn trust(key: &SigningKey) -> RuntimeReleaseTrust {
    RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key.verifying_key())]))
}

#[test]
fn verifies_a_valid_release_at_its_expiry_boundary() {
    let key = SigningKey::from_bytes(&[7; 32]);
    assert!(trust(&key).verify_release(&release(&key), 10).is_ok());
}

#[test]
fn rejects_tampering_unknown_and_revoked_signers() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let mut changed = release(&key);
    changed.payload.rollout_percent = 50;
    assert!(matches!(
        trust(&key).verify_release(&changed, 1),
        Err(RuntimeReleaseTrustError::InvalidSignature)
    ));
    changed = release(&key);
    changed.key_id = "other".into();
    assert!(matches!(
        trust(&key).verify_release(&changed, 1),
        Err(RuntimeReleaseTrustError::UnknownSigner)
    ));
    let mut revoked = trust(&key);
    revoked.revoke_signer("release-1");
    assert!(matches!(
        revoked.verify_release(&release(&key), 1),
        Err(RuntimeReleaseTrustError::RevokedSigner)
    ));
}

#[test]
fn rejects_invalid_signatures_and_signed_unsafe_metadata() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let mut malformed = release(&key);
    malformed.signature_hex = "not hex".into();
    assert!(matches!(
        trust(&key).verify_release(&malformed, 1),
        Err(RuntimeReleaseTrustError::InvalidSignature)
    ));
    let mut expired = release(&key);
    expired.payload.expires_at_unix_seconds = 0;
    expired.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&expired.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        trust(&key).verify_release(&expired, 1),
        Err(RuntimeReleaseTrustError::Expired)
    ));
    let mut revoked = release(&key);
    revoked.payload.revoked = true;
    revoked.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&revoked.payload).unwrap())
            .to_bytes(),
    );
    assert!(matches!(
        trust(&key).verify_release(&revoked, 1),
        Err(RuntimeReleaseTrustError::RevokedRelease)
    ));
}

#[test]
fn rejects_unsupported_and_malformed_signed_manifest_fields() {
    let key = SigningKey::from_bytes(&[7; 32]);
    let mut version = release(&key);
    version.payload.manifest_version = 2;
    assert_invalid_signed_manifest(&key, version);
    let mut rollout = release(&key);
    rollout.payload.rollout_percent = 101;
    assert_invalid_signed_manifest(&key, rollout);
    let mut digest = release(&key);
    digest.payload.artifact.digest_sha256 = "z".repeat(64);
    assert_invalid_signed_manifest(&key, digest);
    let mut artifact = release(&key);
    artifact.payload.artifact.size_bytes = 0;
    assert_invalid_signed_manifest(&key, artifact);
}

fn assert_invalid_signed_manifest(key: &SigningKey, mut release: SignedRuntimeRelease) {
    release.signature_hex = hex::encode(
        key.sign(&serde_json::to_vec(&release.payload).unwrap())
            .to_bytes(),
    );
    assert!(trust(key).verify_release(&release, 1).is_err());
}
