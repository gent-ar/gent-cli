use super::{LocalModelProvisioner, LocalModelProvisioningError, ModelInstallState};
use crate::local_model_catalog::LocalModelCatalog;
use std::fs;
use tempfile::tempdir;

fn provisioner() -> (tempfile::TempDir, LocalModelProvisioner) {
    let directory = tempdir().unwrap();
    let provisioner =
        LocalModelProvisioner::new(directory.path(), LocalModelCatalog::shipped().unwrap());
    (directory, provisioner)
}

fn small_provisioner() -> (tempfile::TempDir, LocalModelProvisioner) {
    let directory = tempdir().unwrap();
    let catalog = LocalModelCatalog::from_json(
        r#"{"models":[{"id":"model","label":"Model","huggingface_url":"https://huggingface.co/gent/model/resolve/0123456789abcdef0123456789abcdef01234567/model.gguf","local_filename":"model.gguf","provider_model_id":"model","size_bytes":5,"sha256":"36bbe50ed96841d10443bcb670d6554f0a34b761be67ec9c4a8ad2c0c44ca42c"}]}"#,
    )
    .unwrap();
    let provisioner = LocalModelProvisioner::new(directory.path(), catalog);
    (directory, provisioner)
}

#[test]
fn catalog_model_gets_a_deterministic_gent_owned_download_plan() {
    let (directory, provisioner) = provisioner();
    let plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    assert_eq!(
        plan.destination,
        directory
            .path()
            .join("models/qwen3-1-7b-q4-k-m/qwen3-1-7b-q4-k-m.gguf")
    );
    assert_eq!(
        plan.partial_destination,
        directory
            .path()
            .join("models/qwen3-1-7b-q4-k-m/qwen3-1-7b-q4-k-m.gguf.part")
    );
    assert!(plan.source_url.starts_with("https://huggingface.co/"));
    assert_eq!(
        provisioner.state(&plan.model_id).unwrap(),
        ModelInstallState::NotInstalled
    );
}

#[test]
fn state_reports_resumable_partial_and_rejects_wrong_sized_final_files() {
    let (_directory, provisioner) = provisioner();
    let plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    provisioner.ensure_storage(&plan).unwrap();
    fs::write(&plan.partial_destination, vec![0_u8; 7]).unwrap();
    assert_eq!(
        provisioner.state(&plan.model_id).unwrap(),
        ModelInstallState::Downloading {
            downloaded_bytes: 7
        }
    );
    fs::remove_file(&plan.partial_destination).unwrap();
    fs::write(&plan.destination, [0_u8; 7]).unwrap();
    assert!(matches!(
        provisioner.state(&plan.model_id),
        Err(LocalModelProvisioningError::UnexpectedFileSize { .. })
    ));
}

#[test]
fn refuses_unknown_and_tampered_download_plans() {
    let (_directory, provisioner) = provisioner();
    assert!(provisioner.plan("../../outside").is_err());
    let mut plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    plan.destination = std::path::PathBuf::from("/tmp/outside.gguf");
    assert!(provisioner.ensure_storage(&plan).is_err());
}

#[test]
fn rejects_incomplete_final_files() {
    let (_directory, provisioner) = provisioner();
    let plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    provisioner.ensure_storage(&plan).unwrap();
    fs::write(&plan.destination, [1_u8]).unwrap();
    assert!(provisioner.state(&plan.model_id).is_err());
}

#[test]
fn rejects_complete_model_files_with_the_wrong_digest() {
    let (_directory, provisioner) = small_provisioner();
    let plan = provisioner.plan("model").unwrap();
    provisioner.ensure_storage(&plan).unwrap();
    fs::write(&plan.destination, b"wrong").unwrap();
    assert!(matches!(
        provisioner.state(&plan.model_id),
        Err(LocalModelProvisioningError::UnexpectedFileDigest { .. })
    ));
}

#[test]
fn recognizes_complete_model_files_with_the_curated_digest() {
    let (_directory, provisioner) = small_provisioner();
    let plan = provisioner.plan("model").unwrap();
    provisioner.ensure_storage(&plan).unwrap();
    fs::write(&plan.destination, b"abcde").unwrap();
    assert_eq!(
        provisioner.state(&plan.model_id).unwrap(),
        ModelInstallState::Ready {
            path: plan.destination
        }
    );
}

#[cfg(unix)]
#[test]
fn rejects_symlinked_model_files() {
    let (_directory, provisioner) = provisioner();
    let plan = provisioner.plan("qwen3-1-7b-q4-k-m").unwrap();
    provisioner.ensure_storage(&plan).unwrap();
    let outside = tempfile::NamedTempFile::new().unwrap();
    std::os::unix::fs::symlink(outside.path(), &plan.destination).unwrap();
    assert!(provisioner.state(&plan.model_id).is_err());
}
