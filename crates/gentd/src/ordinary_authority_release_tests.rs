use std::fs;

use ed25519_dalek::SigningKey;
use sha2::Digest;

use super::fixture::{digest, release, root_keys, runtime};
use super::*;

#[test]
fn one_signed_release_binds_its_nested_authority_to_the_locked_node() {
    let root = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path());
    let signer = SigningKey::from_bytes(&[1; 32]);
    let release = release(&signer, runtime.node_digest_sha256());
    let path = root.path().join("ordinary-authority.json");
    fs::write(&path, serde_json::to_vec(&release).unwrap()).unwrap();
    let verified =
        SignedOrdinaryAuthorityRelease::load_bound(&path, &root_keys(&signer), &runtime, 10)
            .unwrap();
    assert!(matches!(
        verified.providers(),
        [VerifiedProviderAuthority::Codex(_)]
    ));
    assert_eq!(
        verified.compatibility().manifest_sha256(),
        Some(digest(&release.payload.compatibility))
    );
    assert_eq!(
        verified
            .package_policy()
            .approved_package("codex", 10)
            .unwrap()
            .version,
        "0.1.0"
    );
    assert_eq!(
        verified.artifact_digest_sha256(),
        hex::encode(sha2::Sha256::digest(
            serde_json::to_vec(&serde_json::to_value(&release).unwrap()).unwrap()
        ))
    );
}

#[test]
fn unknown_data_and_changed_node_fail_before_any_authority_is_returned() {
    let root = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path());
    let signer = SigningKey::from_bytes(&[2; 32]);
    let release = release(&signer, runtime.node_digest_sha256());
    let path = root.path().join("ordinary-authority.json");
    let mut value = serde_json::to_value(&release).unwrap();
    value
        .as_object_mut()
        .unwrap()
        .insert("unknown".into(), serde_json::json!(true));
    fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
    assert!(matches!(
        SignedOrdinaryAuthorityRelease::load_bound(&path, &root_keys(&signer), &runtime, 10,),
        Err(OrdinaryAuthorityReleaseError::Malformed)
    ));
    fs::write(&path, serde_json::to_vec(&release).unwrap()).unwrap();
    fs::write(root.path().join("node/bin/node"), "changed").unwrap();
    assert!(matches!(
        SignedOrdinaryAuthorityRelease::load_bound(&path, &root_keys(&signer), &runtime, 10,),
        Err(OrdinaryAuthorityReleaseError::EmbeddedAuthority)
    ));
}

#[test]
fn authority_binding_uses_canonical_artifact_content_not_whitespace() {
    let root = tempfile::tempdir().unwrap();
    let runtime = runtime(root.path());
    let signer = SigningKey::from_bytes(&[7; 32]);
    let release = release(&signer, runtime.node_digest_sha256());
    let compact = root.path().join("compact.json");
    let pretty = root.path().join("pretty.json");
    fs::write(&compact, serde_json::to_vec(&release).unwrap()).unwrap();
    fs::write(&pretty, serde_json::to_vec_pretty(&release).unwrap()).unwrap();
    let compact =
        SignedOrdinaryAuthorityRelease::load_bound(&compact, &root_keys(&signer), &runtime, 10)
            .unwrap();
    let pretty =
        SignedOrdinaryAuthorityRelease::load_bound(&pretty, &root_keys(&signer), &runtime, 10)
            .unwrap();
    assert_eq!(
        compact.artifact_digest_sha256(),
        pretty.artifact_digest_sha256()
    );
}
