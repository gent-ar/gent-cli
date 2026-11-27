use super::*;

fn signed_in_provider(directory: &Path) -> (std::path::PathBuf, std::path::PathBuf) {
    let marker = directory.join("authenticated");
    let argv = directory.join("logout-argv");
    let executable = login_script(
        directory,
        &marker,
        &format!(
            "if [ \"$1\" = \"logout\" ] || [ \"$2\" = \"logout\" ]; then echo \"$@\" >> '{argv}'; rm -f '{marker}'; exit 0; fi\ntouch '{marker}'",
            argv = argv.display(),
            marker = marker.display(),
        ),
    );
    fs::write(&marker, "").unwrap();
    (executable, argv)
}

fn logout(port: &StandaloneProviderAuthPort, provider: ProviderAuthProvider) -> ProviderAuthFrame {
    port.exchange(ProviderAuthFrame::LogoutRequest {
        request_id: "logout".into(),
        provider,
    })
    .unwrap()
}

#[test]
fn logout_runs_the_provider_sign_out_and_answers_unauthenticated() {
    for (provider, command) in [
        (ProviderAuthProvider::Claude, "auth logout"),
        (ProviderAuthProvider::Codex, "logout"),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let (executable, argv) = signed_in_provider(directory.path());
        let port = StandaloneProviderAuthPort::for_test(
            crate::provider_executables::ProviderExecutables::explicit(
                Some(executable.clone()),
                Some(executable),
            ),
        );
        assert_eq!(
            status(&port, provider),
            ProviderAuthLifecycle::Authenticated
        );
        let ProviderAuthFrame::Status { request_id, status } = logout(&port, provider) else {
            panic!("logout must answer with a status");
        };
        assert_eq!(request_id, "logout");
        assert_eq!(status.lifecycle, ProviderAuthLifecycle::Unauthenticated);
        assert_eq!(fs::read_to_string(&argv).unwrap().trim(), command);
    }
}

#[test]
fn a_signed_in_provider_signs_in_again_after_logout() {
    let directory = tempfile::tempdir().unwrap();
    let (executable, _) = signed_in_provider(directory.path());
    let port = StandaloneProviderAuthPort::for_test(
        crate::provider_executables::ProviderExecutables::explicit(Some(executable), None),
    );
    assert!(matches!(
        port.exchange(ProviderAuthFrame::LoginRequest {
            request_id: "login".into(),
            provider: ProviderAuthProvider::Claude,
        })
        .unwrap(),
        ProviderAuthFrame::Status { status, .. }
            if status.lifecycle == ProviderAuthLifecycle::Authenticated
    ));
    logout(&port, ProviderAuthProvider::Claude);
    let challenge = offer(&port, ProviderAuthProvider::Claude);
    assert_eq!(select(&port, &challenge), ProviderAuthLifecycle::Verifying);
    assert!(settles_to(
        &port,
        ProviderAuthProvider::Claude,
        ProviderAuthLifecycle::Authenticated
    ));
}

#[test]
fn logout_ends_an_active_sign_in() {
    let directory = tempfile::tempdir().unwrap();
    let marker = directory.path().join("authenticated");
    let executable = login_script(
        directory.path(),
        &marker,
        "if [ \"$1\" = \"logout\" ]; then exit 0; fi\nsleep 20",
    );
    let port = StandaloneProviderAuthPort::for_test(
        crate::provider_executables::ProviderExecutables::explicit(None, Some(executable)),
    );
    let challenge = offer(&port, ProviderAuthProvider::Codex);
    assert_eq!(select(&port, &challenge), ProviderAuthLifecycle::Verifying);
    assert!(matches!(
        logout(&port, ProviderAuthProvider::Codex),
        ProviderAuthFrame::Status { status, .. }
            if status.lifecycle == ProviderAuthLifecycle::Unauthenticated
    ));
    assert!(
        port.exchange(ProviderAuthFrame::Cancel {
            request_id: "cancel".into(),
            challenge_id: challenge.challenge_id,
        })
        .is_err()
    );
}

#[test]
fn logout_of_a_missing_provider_reports_not_installed() {
    let port = StandaloneProviderAuthPort::for_test(
        crate::provider_executables::ProviderExecutables::explicit(None, None),
    );
    assert!(matches!(
        logout(&port, ProviderAuthProvider::Codex),
        ProviderAuthFrame::Status { status, .. }
            if status.lifecycle == ProviderAuthLifecycle::NotInstalled && status.binary_lock.is_none()
    ));
}
