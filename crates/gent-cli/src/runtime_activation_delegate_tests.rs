use super::*;
use command::IdentityPin;

#[test]
fn flagless_capability_probe_requires_identity_before_any_process() {
    let error = execute(
        RuntimeCommand::ActivationCapabilities(IdentityPin {
            expected_maintenance_daemon_sha256: None,
        }),
        PathBuf::from("not-created"),
    )
    .unwrap_err();
    assert!(error.contains("maintenanceIdentityRequired"));
    assert!(!std::path::Path::new("not-created").exists());
}
#[test]
fn pinned_identity_rejects_different_bytes_without_executing_them() {
    use sha2::{Digest, Sha256};
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().canonicalize().unwrap().join("gentd");
    std::fs::write(&path, b"non executable fixture").unwrap();
    let digest =
        Sha256::digest(b"non executable fixture")
            .iter()
            .fold(String::new(), |mut hex, byte| {
                use std::fmt::Write;
                let _ = write!(hex, "{byte:02x}");
                hex
            });
    delegate::verify(&path, &digest).unwrap();
    std::fs::write(&path, b"replaced fixture").unwrap();
    assert!(
        delegate::verify(&path, &digest)
            .unwrap_err()
            .contains("maintenanceIdentityMismatch")
    );
}
#[test]
fn bounded_output_rejects_large_reply_without_storing_unbounded_bytes() {
    let bytes = vec![0; ACTIVATION_MAINTENANCE_MAX_BYTES + 1];
    assert!(delegate::bounded_output(bytes.as_slice()).is_err());
    assert_eq!(delegate::bounded_output(b"{}".as_slice()).unwrap(), b"{}");
}
