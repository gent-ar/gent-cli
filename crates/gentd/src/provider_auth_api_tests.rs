#[cfg(unix)]
use std::{fs, os::unix::fs::PermissionsExt, path::Path, thread, time::Duration};

#[cfg(unix)]
use gent_types::{ProviderAuthChallenge, ProviderAuthMethod};

use gent_protocol::{PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame};
use gent_runtime::catalog::{
    RuntimeCapabilityFeature, RuntimeCapabilityProfile, declared_capabilities_with_profiles,
};
use gent_types::{ProviderAuthLifecycle, ProviderAuthMethodSelection, ProviderAuthProvider};

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
}

#[test]
fn missing_provider_status_is_not_installed_without_a_fake_binary_lock() {
    let port = StandaloneProviderAuthPort::new(
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
    let port = StandaloneProviderAuthPort::new(
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
    let port = StandaloneProviderAuthPort::new(
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
fn provider_script(directory: &Path, marker: &Path, slow: bool) -> std::path::PathBuf {
    let delay = if slow { "sleep 20" } else { ":" };
    login_script(
        directory,
        marker,
        &format!("{delay}\ntouch '{}'", marker.display()),
    )
}

#[cfg(unix)]
fn login_script(directory: &Path, marker: &Path, login: &str) -> std::path::PathBuf {
    let executable = directory.join("provider");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'provider 1.2.3'; exit 0; fi\nif [ \"$1\" = \"auth\" ] && [ \"$2\" = \"status\" ]; then if [ -f '{marker}' ]; then echo '{{\"loggedIn\":true}}'; exit 0; fi; echo '{{\"loggedIn\":false}}'; exit 1; fi\nif [ \"$1\" = \"login\" ] && [ \"$2\" = \"status\" ]; then if [ -f '{marker}' ]; then echo 'Logged in using ChatGPT'; exit 0; fi; echo 'Not logged in' >&2; exit 1; fi\n{login}\n",
            marker = marker.display(),
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&executable, permissions).unwrap();
    crate::provider_launch_budget::warm_first_execution(&executable);
    executable
}

#[cfg(unix)]
fn status(
    port: &StandaloneProviderAuthPort,
    provider: ProviderAuthProvider,
) -> ProviderAuthLifecycle {
    let ProviderAuthFrame::Status { status, .. } = port
        .exchange(ProviderAuthFrame::StatusRequest {
            request_id: "status".into(),
            provider,
        })
        .unwrap()
    else {
        panic!("status request must return status");
    };
    status.lifecycle
}

#[cfg(unix)]
fn offer(
    port: &StandaloneProviderAuthPort,
    provider: ProviderAuthProvider,
) -> ProviderAuthChallenge {
    let ProviderAuthFrame::AskTool { challenge, .. } = port
        .exchange(ProviderAuthFrame::LoginRequest {
            request_id: "login".into(),
            provider,
        })
        .unwrap()
    else {
        panic!("login must offer a provider-auth challenge");
    };
    assert_eq!(challenge.methods, [ProviderAuthMethod::AccountBrowser]);
    challenge
}

#[cfg(unix)]
fn select(
    port: &StandaloneProviderAuthPort,
    challenge: &ProviderAuthChallenge,
) -> ProviderAuthLifecycle {
    let ProviderAuthFrame::SelectionAccepted { status, .. } = port
        .exchange(ProviderAuthFrame::SelectMethod {
            request_id: "select".into(),
            selection: ProviderAuthMethodSelection {
                challenge_id: challenge.challenge_id.clone(),
                method: ProviderAuthMethod::AccountBrowser,
            },
        })
        .unwrap()
    else {
        panic!("method selection must be accepted");
    };
    status.lifecycle
}

#[cfg(unix)]
fn settles_to(
    port: &StandaloneProviderAuthPort,
    provider: ProviderAuthProvider,
    expected: ProviderAuthLifecycle,
) -> bool {
    (0..300).any(|_| {
        let settled = status(port, provider) == expected;
        if !settled {
            thread::sleep(Duration::from_millis(10));
        }
        settled
    })
}

#[cfg(unix)]
#[test]
fn installed_provider_reports_unauthenticated_then_an_offered_challenge() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("authenticated");
    let executable = provider_script(directory.path(), &marker, false);
    let port = StandaloneProviderAuthPort::new(
        crate::provider_executables::ProviderExecutables::explicit(Some(executable), None),
    );
    assert_eq!(
        status(&port, ProviderAuthProvider::Claude),
        ProviderAuthLifecycle::Unauthenticated
    );
    offer(&port, ProviderAuthProvider::Claude);
    assert_eq!(
        status(&port, ProviderAuthProvider::Claude),
        ProviderAuthLifecycle::ChallengeOffered
    );
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn an_unanswered_challenge_reports_expired_on_the_next_status_request() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("authenticated");
    let executable = provider_script(directory.path(), &marker, false);
    let port = StandaloneProviderAuthPort::new(
        crate::provider_executables::ProviderExecutables::explicit(None, Some(executable)),
    )
    .with_windows(Duration::ZERO, Duration::from_secs(600));
    let challenge = offer(&port, ProviderAuthProvider::Codex);
    assert_eq!(
        status(&port, ProviderAuthProvider::Codex),
        ProviderAuthLifecycle::Expired
    );
    assert!(
        port.exchange(ProviderAuthFrame::SelectMethod {
            request_id: "select-expired".into(),
            selection: ProviderAuthMethodSelection {
                challenge_id: challenge.challenge_id,
                method: ProviderAuthMethod::AccountBrowser,
            },
        })
        .is_err()
    );
    assert!(!marker.exists());
}

#[cfg(unix)]
#[test]
fn a_login_process_past_its_bound_is_killed_and_timed_out() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("authenticated");
    let executable = login_script(
        directory.path(),
        &marker,
        &format!("sleep 1\ntouch '{}'", marker.display()),
    );
    let port = StandaloneProviderAuthPort::new(
        crate::provider_executables::ProviderExecutables::explicit(Some(executable), None),
    )
    .with_windows(Duration::from_secs(600), Duration::from_millis(200));
    let challenge = offer(&port, ProviderAuthProvider::Claude);
    assert_eq!(select(&port, &challenge), ProviderAuthLifecycle::Verifying);
    thread::sleep(Duration::from_millis(1600));
    assert!(
        !marker.exists(),
        "the daemon killed the login process at its bound without a status request"
    );
    assert_eq!(
        status(&port, ProviderAuthProvider::Claude),
        ProviderAuthLifecycle::TimedOut
    );
}

#[cfg(unix)]
#[test]
fn a_login_process_that_exits_unsuccessfully_is_failed() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("authenticated");
    let executable = login_script(directory.path(), &marker, "echo 'secret-token' ; exit 1");
    let port = StandaloneProviderAuthPort::new(
        crate::provider_executables::ProviderExecutables::explicit(None, Some(executable)),
    );
    let challenge = offer(&port, ProviderAuthProvider::Codex);
    assert_eq!(select(&port, &challenge), ProviderAuthLifecycle::Verifying);
    assert!(settles_to(
        &port,
        ProviderAuthProvider::Codex,
        ProviderAuthLifecycle::Failed
    ));
    let frame = port
        .exchange(ProviderAuthFrame::StatusRequest {
            request_id: "status-failed".into(),
            provider: ProviderAuthProvider::Codex,
        })
        .unwrap();
    assert!(
        !serde_json::to_string(&frame)
            .unwrap()
            .contains("secret-token")
    );
}
