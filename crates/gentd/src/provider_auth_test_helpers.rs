use super::*;

#[cfg(unix)]
pub(super) fn provider_script(directory: &Path, marker: &Path, slow: bool) -> std::path::PathBuf {
    let delay = if slow { "sleep 20" } else { ":" };
    login_script(
        directory,
        marker,
        &format!("{delay}\ntouch '{}'", marker.display()),
    )
}

#[cfg(unix)]
pub(super) fn login_script(directory: &Path, marker: &Path, login: &str) -> std::path::PathBuf {
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
pub(super) fn status(
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
pub(super) fn offer(
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
pub(super) fn select(
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
pub(super) fn settles_to(
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
