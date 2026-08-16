#![cfg(not(windows))]

use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    process::Command,
};

use ed25519_dalek::VerifyingKey;
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
    let private = directory.path().join("private.pem");
    let public = directory.path().join("public.pem");
    let archive = directory.path().join("archive.manifest.json");
    let output = directory.path().join("runtime-release.json");
    command(
        "openssl",
        &[
            "genpkey",
            "-algorithm",
            "ED25519",
            "-out",
            private.to_str().unwrap(),
        ],
    );
    command(
        "openssl",
        &[
            "pkey",
            "-in",
            private.to_str().unwrap(),
            "-pubout",
            "-out",
            public.to_str().unwrap(),
        ],
    );
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
    let der = Command::new("openssl")
        .args([
            "pkey",
            "-pubin",
            "-in",
            public.to_str().unwrap(),
            "-pubout",
            "-outform",
            "DER",
        ])
        .output()
        .unwrap();
    assert!(der.status.success());
    let raw: [u8; 32] = der.stdout[der.stdout.len() - 32..].try_into().unwrap();
    let trust = RuntimeReleaseTrust::new(BTreeMap::from([(
        "release-1".into(),
        VerifyingKey::from_bytes(&raw).unwrap(),
    )]));
    trust.verify_release(&release, 1).unwrap();
}
