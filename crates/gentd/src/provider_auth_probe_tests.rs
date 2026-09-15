use gent_types::{ProviderAuthLifecycle, ProviderAuthProvider};

use super::classify;

#[test]
fn claude_status_json_decides_sign_in_independently_of_its_exit_code() {
    assert_eq!(
        classify(
            ProviderAuthProvider::Claude,
            Some(0),
            br#"{"loggedIn":true}"#,
            b""
        ),
        ProviderAuthLifecycle::Authenticated
    );
    assert_eq!(
        classify(
            ProviderAuthProvider::Claude,
            Some(1),
            br#"{"loggedIn":false,"authMethod":"none"}"#,
            b""
        ),
        ProviderAuthLifecycle::Unauthenticated
    );
}

#[test]
fn codex_reports_signed_out_only_for_its_documented_status_message() {
    assert_eq!(
        classify(
            ProviderAuthProvider::Codex,
            Some(0),
            b"Logged in using ChatGPT\n",
            b""
        ),
        ProviderAuthLifecycle::Authenticated
    );
    assert_eq!(
        classify(
            ProviderAuthProvider::Codex,
            Some(1),
            b"",
            b"Not logged in\n"
        ),
        ProviderAuthLifecycle::Unauthenticated
    );
}

#[test]
fn provider_cli_errors_are_failures_not_signed_out() {
    for provider in [ProviderAuthProvider::Claude, ProviderAuthProvider::Codex] {
        for (code, stdout, stderr) in [
            (Some(2), &b""[..], &b"error: unknown command 'status'\n"[..]),
            (Some(3), b"token=abc", b"panic"),
            (None, b"", b""),
        ] {
            assert_eq!(
                classify(provider, code, stdout, stderr),
                ProviderAuthLifecycle::Failed
            );
        }
    }
    assert_eq!(
        classify(ProviderAuthProvider::Claude, Some(0), b"not json", b""),
        ProviderAuthLifecycle::Failed
    );
}

#[cfg(unix)]
#[test]
fn a_failing_status_command_is_reported_as_failed_and_rechecked() {
    use std::{fs, os::unix::fs::PermissionsExt};

    use gent_protocol::ProviderAuthFrame;

    use crate::provider_auth_api::{ProviderAuthPort, StandaloneProviderAuthPort};

    let directory = tempfile::tempdir().unwrap();
    let broken = directory.path().join("broken");
    let executable = directory.path().join("provider");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nif [ \"$1\" = \"--version\" ]; then echo 'provider 1.2.3'; exit 0; fi\nif [ -f '{}' ]; then echo 'secret-token' >&2; exit 3; fi\necho 'Not logged in' >&2\nexit 1\n",
            broken.display()
        ),
    )
    .unwrap();
    fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
    crate::provider_launch_budget::warm_first_execution(&executable);
    fs::write(&broken, "").unwrap();
    let port = StandaloneProviderAuthPort::new(
        crate::provider_executables::ProviderExecutables::explicit(None, Some(executable)),
    );
    let lifecycle = |port: &StandaloneProviderAuthPort| match port
        .exchange(ProviderAuthFrame::StatusRequest {
            request_id: "status".into(),
            provider: ProviderAuthProvider::Codex,
        })
        .unwrap()
    {
        ProviderAuthFrame::Status { status, .. } => status.lifecycle,
        _ => panic!("status request must return status"),
    };
    assert_eq!(lifecycle(&port), ProviderAuthLifecycle::Failed);
    fs::remove_file(&broken).unwrap();
    assert_eq!(lifecycle(&port), ProviderAuthLifecycle::Unauthenticated);
}
