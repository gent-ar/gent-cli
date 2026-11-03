use super::*;

#[test]
fn cached_identity_uses_actual_executable_and_acquired_physical_data_root() {
    let directory = tempfile::tempdir().unwrap();
    let held = crate::host_lock::acquire(directory.path()).unwrap();
    let captured = capture(&held).unwrap();
    assert_eq!(
        captured.canonical_executable_path,
        std::env::current_exe().unwrap().canonicalize().unwrap()
    );
    assert_eq!(
        captured.canonical_data_directory,
        directory.path().canonicalize().unwrap()
    );
    assert_eq!(captured.process_id, std::process::id());
    assert!(
        !uuid::Uuid::parse_str(&captured.boot_nonce)
            .unwrap()
            .is_nil()
    );
    let first = identity_frame(&captured, "first".into(), HostEpoch(4));
    let second = identity_frame(&captured, "second".into(), HostEpoch(4));
    let RuntimeOwnerFrame::RuntimeOwnerIdentityState {
        request_id,
        host_epoch,
        canonical_executable_path,
        canonical_data_directory,
        boot_nonce,
        executable_digest_sha256,
        ..
    } = first
    else {
        panic!()
    };
    assert_eq!(request_id, "first");
    assert_eq!(host_epoch, HostEpoch(4));
    assert_eq!(
        canonical_executable_path,
        captured.canonical_executable_path
    );
    assert_eq!(canonical_data_directory, captured.canonical_data_directory);
    assert_eq!(executable_digest_sha256, captured.executable_digest_sha256);
    let RuntimeOwnerFrame::RuntimeOwnerIdentityState {
        request_id,
        boot_nonce: same_nonce,
        ..
    } = second
    else {
        panic!()
    };
    assert_eq!(request_id, "second");
    assert_eq!(boot_nonce, same_nonce);
}

#[test]
fn same_hash_different_root_and_changed_instance_are_not_the_cached_identity() {
    let directory = tempfile::tempdir().unwrap();
    let held = crate::host_lock::acquire(directory.path()).unwrap();
    let captured = capture(&held).unwrap();
    let mut wrong_root = captured.clone();
    wrong_root.canonical_executable_path = directory.path().join("other-release").join("gentd.exe");
    assert_eq!(
        wrong_root.executable_digest_sha256,
        captured.executable_digest_sha256
    );
    assert_ne!(
        identity_frame(&captured, "request".into(), HostEpoch(4)),
        identity_frame(&wrong_root, "request".into(), HostEpoch(4))
    );
    let mut wrong_data = captured.clone();
    wrong_data.canonical_data_directory = directory.path().join("other-data");
    assert_eq!(
        wrong_data.executable_digest_sha256,
        captured.executable_digest_sha256
    );
    assert_ne!(
        identity_frame(&captured, "request".into(), HostEpoch(4)),
        identity_frame(&wrong_data, "request".into(), HostEpoch(4))
    );
    let replacement_instance = capture(&held).unwrap();
    assert_eq!(replacement_instance.process_id, captured.process_id);
    assert_eq!(
        replacement_instance.canonical_executable_path,
        captured.canonical_executable_path
    );
    assert_ne!(replacement_instance.boot_nonce, captured.boot_nonce);
}

#[test]
fn uncaptured_identity_is_explicitly_unavailable_without_caller_path_substitution() {
    assert_eq!(
        frame_from(None, "correlated".into(), HostEpoch(4)),
        RuntimeOwnerFrame::RuntimeOwnerIdentityUnavailable {
            request_id: "correlated".into()
        }
    );
}
