use gent_core::{ProviderAuthEvent, ProviderAuthState, reduce_provider_auth};
use gent_protocol::ProviderAuthFrame;
use gent_types::{
    ProviderAuthChallenge, ProviderAuthLifecycle, ProviderAuthMethod, ProviderAuthMethodSelection,
    ProviderAuthProvider, RunVersionLock,
};

use super::{
    ProviderAuthPort, StandaloneProviderAuthPort, provider_auth_process::public_lock,
    provider_auth_session::ProviderAuthSession,
};

#[test]
fn successful_login_exit_cannot_stay_verifying_after_status_probe_timeout_forever() {
    use super::provider_auth_session::completed_login_lifecycle;
    let timeout = || {
        Err(crate::provider_launch_budget::ProviderLaunchError::TimedOut("bounded probe".into()))
    };
    assert_eq!(completed_login_lifecycle(timeout(), false), None);
    assert_eq!(
        completed_login_lifecycle(timeout(), true),
        Some(ProviderAuthLifecycle::TimedOut)
    );
    assert_eq!(
        completed_login_lifecycle(Ok(ProviderAuthLifecycle::Authenticated), true),
        Some(ProviderAuthLifecycle::Authenticated)
    );
    assert_eq!(
        completed_login_lifecycle(Ok(ProviderAuthLifecycle::Unauthenticated), false),
        Some(ProviderAuthLifecycle::Failed)
    );
}

#[test]
fn a_live_login_finishes_only_after_authoritative_signed_in_status() {
    use super::provider_auth_session::live_login_lifecycle;
    assert_eq!(
        live_login_lifecycle(Ok(ProviderAuthLifecycle::Authenticated), false),
        Some(ProviderAuthLifecycle::Authenticated)
    );
    assert_eq!(
        live_login_lifecycle(Ok(ProviderAuthLifecycle::Unauthenticated), false),
        None
    );
    assert_eq!(
        live_login_lifecycle(Ok(ProviderAuthLifecycle::Failed), false),
        None
    );
    assert_eq!(
        live_login_lifecycle(
            Err(
                crate::provider_launch_budget::ProviderLaunchError::TimedOut(
                    "bounded probe".into()
                )
            ),
            false
        ),
        None
    );
    assert_eq!(
        live_login_lifecycle(Ok(ProviderAuthLifecycle::Unauthenticated), true),
        Some(ProviderAuthLifecycle::TimedOut)
    );
}

#[test]
fn status_reopens_the_exact_active_challenge_and_cancel_remains_bound_to_it() {
    let port = StandaloneProviderAuthPort::for_test(
        crate::provider_executables::ProviderExecutables::explicit(None, None),
    );
    let provider = ProviderAuthProvider::Claude;
    let lock = RunVersionLock {
        provider: "claude".into(),
        canonical_path: "not-executed".into(),
        file_identity: "fake".into(),
        digest_sha256: "a".repeat(64),
        version: "1".into(),
        compatibility_entry: "fake".into(),
    };
    let challenge = ProviderAuthChallenge {
        challenge_id: "challenge-active".into(),
        provider,
        binary_lock: public_lock(provider, &lock),
        methods: vec![ProviderAuthMethod::AccountBrowser],
        expires_at_unix_seconds: u64::MAX,
    };
    let (state, _) = reduce_provider_auth(
        ProviderAuthState::default(),
        ProviderAuthEvent::ObservedUnauthenticated { challenge },
    );
    let (state, _) = reduce_provider_auth(
        state,
        ProviderAuthEvent::SelectMethod {
            selection: ProviderAuthMethodSelection {
                challenge_id: "challenge-active".into(),
                method: ProviderAuthMethod::AccountBrowser,
            },
            now: 1,
        },
    );
    super::recover(&port.sessions).push(ProviderAuthSession {
        provider,
        state,
        lock,
        login: None,
        device_login: None,
        lifecycle_control: port.lifecycle_control.clone(),
    });
    let status = port
        .exchange(ProviderAuthFrame::StatusRequest {
            request_id: "fresh-card".into(),
            provider,
        })
        .unwrap();
    let ProviderAuthFrame::Status { status, .. } = status else {
        panic!("expected status")
    };
    assert_eq!(status.lifecycle, ProviderAuthLifecycle::Verifying);
    assert_eq!(status.challenge_id.as_deref(), Some("challenge-active"));
    let wire = serde_json::to_value(&status).unwrap();
    assert_eq!(wire["challengeId"], "challenge-active");
    assert!(
        port.exchange(ProviderAuthFrame::Cancel {
            request_id: "stale".into(),
            challenge_id: "challenge-other".into()
        })
        .is_err()
    );
    let other = port.check_status(ProviderAuthProvider::Codex);
    assert_eq!(other.lifecycle, ProviderAuthLifecycle::NotInstalled);
    assert!(other.challenge_id.is_none());
    let cancelled = port
        .exchange(ProviderAuthFrame::Cancel {
            request_id: "cancel".into(),
            challenge_id: status.challenge_id.unwrap(),
        })
        .unwrap();
    assert!(matches!(cancelled, ProviderAuthFrame::Status { status, .. }
        if status.lifecycle == ProviderAuthLifecycle::Cancelled && status.challenge_id.is_none()));
}
