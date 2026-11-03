use super::*;

#[cfg(unix)]
#[test]
fn installed_provider_reports_unauthenticated_then_an_offered_challenge() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("authenticated");
    let executable = provider_script(directory.path(), &marker, false);
    let port = StandaloneProviderAuthPort::for_test(
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
    let port = StandaloneProviderAuthPort::for_test(
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
    let port = StandaloneProviderAuthPort::for_test(
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
    let port = StandaloneProviderAuthPort::for_test(
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
