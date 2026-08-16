#![cfg(not(windows))]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

use ed25519_dalek::SigningKey;
use gent_runtime::RuntimeReleaseTrust;
use gent_types::SignedRuntimeRelease;
use tempfile::tempdir;

fn command(program: &str, arguments: &[&str]) {
    let output = Command::new(program).args(arguments).output().unwrap();
    assert!(
        output.status.success(),
        "{program} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

#[test]
fn release_signer_emits_a_manifest_the_runtime_revalidates() {
    let directory = tempdir().unwrap();
    let private = directory.path().join("private.seed");
    let archive = directory.path().join("archive.manifest.json");
    let output = directory.path().join("runtime-release.json");
    let seed = [7; 32];
    std::fs::write(&private, seed).unwrap();
    std::fs::write(
        &archive,
        r#"{"schemaVersion":1,"version":"v2.0.0","target":"fixture-target","archive":{"name":"gent.tar.gz","sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","size":1}}"#,
    )
    .unwrap();
    command(
        "python3",
        &[
            root()
                .join("tools/sign-runtime-release.py")
                .to_str()
                .unwrap(),
            "--archive-manifest",
            archive.to_str().unwrap(),
            "--version",
            "v2.0.0",
            "--target",
            "fixture-target",
            "--key-id",
            "release-1",
            "--private-key",
            private.to_str().unwrap(),
            "--expires-at",
            "4102444800",
            "--schema-max",
            "22",
            "--out",
            output.to_str().unwrap(),
        ],
    );
    let release: SignedRuntimeRelease =
        serde_json::from_slice(&std::fs::read(&output).unwrap()).unwrap();
    let key = SigningKey::from_bytes(&seed).verifying_key();
    let trust = RuntimeReleaseTrust::new(BTreeMap::from([("release-1".into(), key)]));
    trust.verify_release(&release, 1).unwrap();
}
