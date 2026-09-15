use super::{file_sha256, matches_verified_sha256, remember_sha256, verification_path};
use std::{fs, path::PathBuf};

fn model(bytes: &[u8]) -> (tempfile::TempDir, PathBuf, String) {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("model.gguf");
    fs::write(&path, bytes).unwrap();
    let digest = file_sha256(&path).unwrap();
    (directory, path, digest)
}

#[test]
fn first_verification_retains_a_proof_for_the_exact_file_identity() {
    let (_directory, path, digest) = model(b"curated weights");
    assert!(!verification_path(&path).exists());
    assert!(matches_verified_sha256(&path, &digest).unwrap());
    let retained = fs::read_to_string(verification_path(&path)).unwrap();
    assert!(retained.contains(&digest));
    assert!(retained.contains("\"sizeBytes\":15"));
    assert!(matches_verified_sha256(&path, &digest).unwrap());
}

#[test]
fn a_retained_proof_answers_without_reading_the_model_bytes() {
    let (_directory, path, digest) = model(b"curated weights");
    let unread = "a".repeat(64);
    remember_sha256(&path, &unread);
    assert_ne!(digest, unread);
    assert!(matches_verified_sha256(&path, &unread).unwrap());
}

#[test]
fn a_truncated_file_is_rejected_and_its_stale_proof_is_discarded() {
    let (_directory, path, digest) = model(b"curated weights");
    assert!(matches_verified_sha256(&path, &digest).unwrap());
    fs::write(&path, b"curated").unwrap();
    assert!(!matches_verified_sha256(&path, &digest).unwrap());
    assert!(!verification_path(&path).exists());
}

#[test]
fn a_same_size_tampered_file_is_rejected_even_though_a_proof_was_retained() {
    let (_directory, path, digest) = model(b"curated weights");
    assert!(matches_verified_sha256(&path, &digest).unwrap());
    fs::write(&path, b"tampered wights").unwrap();
    assert!(!matches_verified_sha256(&path, &digest).unwrap());
}

#[test]
fn a_proof_for_one_digest_never_proves_a_different_curated_digest() {
    let (_directory, path, digest) = model(b"curated weights");
    remember_sha256(&path, &"a".repeat(64));
    assert!(matches_verified_sha256(&path, &digest).unwrap());
    assert!(
        fs::read_to_string(verification_path(&path))
            .unwrap()
            .contains(&digest)
    );
}

#[test]
fn a_corrupt_proof_falls_back_to_hashing_the_file() {
    let (_directory, path, digest) = model(b"curated weights");
    fs::write(verification_path(&path), b"not json").unwrap();
    assert!(matches_verified_sha256(&path, &digest).unwrap());
    assert!(!matches_verified_sha256(&path, &"b".repeat(64)).unwrap());
}

#[test]
fn a_missing_model_file_is_an_error_rather_than_a_retained_answer() {
    let (_directory, path, digest) = model(b"curated weights");
    assert!(matches_verified_sha256(&path, &digest).unwrap());
    fs::remove_file(&path).unwrap();
    assert!(matches_verified_sha256(&path, &digest).is_err());
}
