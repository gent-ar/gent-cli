#[cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::Path, thread, time::Duration};

#[cfg(unix)]
use gent_types::{ProviderAuthChallenge, ProviderAuthMethod};

use gent_protocol::{PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame};
use gent_runtime::catalog::{
    RuntimeCapabilityFeature, RuntimeCapabilityProfile, declared_capabilities_with_profiles,
};
#[cfg(unix)]
use gent_types::ProviderAuthMethodSelection;
use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider};

use super::{ProviderAuthPort, StandaloneProviderAuthPort};

#[test]
fn provider_auth_requires_an_explicit_capability_profile() {
    let observer = declared_capabilities_with_profiles(&RuntimeCapabilityProfile::default());
    assert!(!observer.0.contains(&PROVIDER_AUTH_CAPABILITY.into()));
    let standalone = declared_capabilities_with_profiles(&RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::ProviderAuth,
    ]));
    assert!(standalone.0.contains(&PROVIDER_AUTH_CAPABILITY.into()));
    assert!(
        !standalone
            .0
            .contains(&gent_protocol::PROVIDER_AUTH_HANDOFF_CAPABILITY.into())
    );
    let handoff = declared_capabilities_with_profiles(&RuntimeCapabilityProfile::new([
        RuntimeCapabilityFeature::AgentChat,
        RuntimeCapabilityFeature::ProviderAuth,
        RuntimeCapabilityFeature::ProviderAuthHandoff,
    ]));
    assert!(
        handoff
            .0
            .contains(&gent_protocol::PROVIDER_AUTH_HANDOFF_CAPABILITY.into())
    );
}

#[test]
fn missing_provider_status_is_not_installed_without_a_fake_binary_lock() {
    let port = StandaloneProviderAuthPort::for_test(
        crate::provider_executables::ProviderExecutables::explicit(None, None),
    );
    let frame = port
        .exchange(ProviderAuthFrame::StatusRequest {
            request_id: "status-1".into(),
            provider: ProviderAuthProvider::Claude,
        })
        .unwrap();
    assert!(matches!(
        frame,
        ProviderAuthFrame::Status { status, .. }
            if status.lifecycle == ProviderAuthLifecycle::NotInstalled && status.binary_lock.is_none()
    ));
    let login = port
        .exchange(ProviderAuthFrame::LoginRequest {
            request_id: "login-missing".into(),
            provider: ProviderAuthProvider::Claude,
        })
        .unwrap();
    assert!(matches!(
        login,
        ProviderAuthFrame::Status { status, .. }
            if status.lifecycle == ProviderAuthLifecycle::NotInstalled && status.binary_lock.is_none()
    ));
}

#[cfg(unix)]
#[test]
fn browser_login_is_owned_and_observed_by_the_daemon_port() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("authenticated");
    let executable = provider_script(directory.path(), &marker, false);
    let port = StandaloneProviderAuthPort::for_test(
        crate::provider_executables::ProviderExecutables::explicit(
            Some(executable.clone()),
            Some(executable),
        ),
    );
    let challenge = port
        .exchange(ProviderAuthFrame::LoginRequest {
            request_id: "login-1".into(),
            provider: ProviderAuthProvider::Claude,
        })
        .unwrap();
    let ProviderAuthFrame::AskTool { challenge, .. } = challenge else {
        panic!("expected provider-auth challenge");
    };
    assert_eq!(
        challenge.methods,
        [gent_types::ProviderAuthMethod::AccountBrowser]
    );
    let accepted = port
        .exchange(ProviderAuthFrame::SelectMethod {
            request_id: "select-1".into(),
            selection: ProviderAuthMethodSelection {
                challenge_id: challenge.challenge_id,
                method: gent_types::ProviderAuthMethod::AccountBrowser,
            },
        })
        .unwrap();
    assert!(matches!(
        accepted,
        ProviderAuthFrame::SelectionAccepted { .. }
    ));
    let mut authenticated = false;
    for _ in 0..100 {
        let status = port
            .exchange(ProviderAuthFrame::StatusRequest {
                request_id: "status-2".into(),
                provider: ProviderAuthProvider::Claude,
            })
            .unwrap();
        if matches!(
            status,
            ProviderAuthFrame::Status { status, .. }
                if status.lifecycle == ProviderAuthLifecycle::Authenticated
        ) {
            authenticated = true;
            break;
        }
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        authenticated,
        "provider authentication did not become authenticated"
    );
    fs::remove_file(&marker).unwrap();
    let status = port
        .exchange(ProviderAuthFrame::StatusRequest {
            request_id: "status-3".into(),
            provider: ProviderAuthProvider::Claude,
        })
        .unwrap();
    assert!(matches!(
        status,
        ProviderAuthFrame::Status { status, .. }
            if status.lifecycle == ProviderAuthLifecycle::Unauthenticated
    ));
}

#[cfg(unix)]
#[test]
fn cancellation_terminates_the_owned_login_without_exposing_process_output() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("authenticated");
    let executable = provider_script(directory.path(), &marker, true);
    let port = StandaloneProviderAuthPort::for_test(
        crate::provider_executables::ProviderExecutables::explicit(
            Some(executable.clone()),
            Some(executable),
        ),
    );
    let challenge = port
        .exchange(ProviderAuthFrame::LoginRequest {
            request_id: "login-2".into(),
            provider: ProviderAuthProvider::Codex,
        })
        .unwrap();
    let ProviderAuthFrame::AskTool { challenge, .. } = challenge else {
        panic!("expected provider-auth challenge");
    };
    let challenge_id = challenge.challenge_id;
    port.exchange(ProviderAuthFrame::SelectMethod {
        request_id: "select-2".into(),
        selection: ProviderAuthMethodSelection {
            challenge_id: challenge_id.clone(),
            method: gent_types::ProviderAuthMethod::AccountBrowser,
        },
    })
    .unwrap();
    let cancelled = port
        .exchange(ProviderAuthFrame::Cancel {
            request_id: "cancel-1".into(),
            challenge_id,
        })
        .unwrap();
    assert!(matches!(
        cancelled,
        ProviderAuthFrame::Status { status, .. }
            if status.lifecycle == ProviderAuthLifecycle::Cancelled
    ));
    assert!(!marker.exists());
}

#[cfg(unix)]
#[path = "provider_auth_test_helpers.rs"]
mod helpers;
#[cfg(unix)]
use helpers::*;
#[cfg(unix)]
#[path = "provider_auth_lifecycle_tests.rs"]
mod lifecycle;
