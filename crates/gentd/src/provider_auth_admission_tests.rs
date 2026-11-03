use super::{ProviderAuthPort, StandaloneProviderAuthPort};
use gent_protocol::ProviderAuthFrame;
use gent_types::ProviderAuthProvider;

#[test]
fn detached_auth_port_cannot_start_any_native_child() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("unexecuted-provider.exe");
    std::fs::write(&executable, b"must not execute").unwrap();
    let port = StandaloneProviderAuthPort::new(
        crate::provider_executables::ProviderExecutables::explicit(None, Some(executable)),
    );
    let result = port.exchange(ProviderAuthFrame::HandoffLoginRequest {
        request_id: "detached".into(),
        provider: ProviderAuthProvider::Codex,
    });
    assert!(
        matches!(result,Err(message) if message=="provider authentication admission is unavailable")
    );
    assert!(super::recover(&port.sessions).is_empty());
}

#[test]
fn draining_auth_port_cannot_launch_a_version_probe_or_offer_challenge() {
    let directory = tempfile::tempdir().unwrap();
    let executable = directory.path().join("unexecuted-provider.exe");
    std::fs::write(&executable, b"must not execute").unwrap();
    let control = crate::ordinary_lifecycle_control::OrdinaryLifecycleControl::new();
    control.open_after_recovery();
    control.request_shutdown();
    let port = StandaloneProviderAuthPort::new(
        crate::provider_executables::ProviderExecutables::explicit(None, Some(executable)),
    )
    .with_lifecycle_control(control);
    assert!(
        port.exchange(ProviderAuthFrame::HandoffLoginRequest {
            request_id: "draining".into(),
            provider: ProviderAuthProvider::Codex
        })
        .is_err()
    );
    assert!(super::recover(&port.sessions).is_empty());
}
