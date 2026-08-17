use gent_types::{
    ProviderAuthBinaryLock, ProviderAuthChallenge, ProviderAuthLifecycle, ProviderAuthMethod,
    ProviderAuthMethodSelection, ProviderAuthProvider,
};

use super::{
    ProviderAuthEffect, ProviderAuthEvent, ProviderAuthRejection, ProviderAuthState,
    reduce_provider_auth,
};

fn challenge() -> ProviderAuthChallenge {
    ProviderAuthChallenge {
        challenge_id: "challenge-1".into(),
        provider: ProviderAuthProvider::Codex,
        binary_lock: ProviderAuthBinaryLock {
            canonical_executable_id: "provider-id:codex".into(),
            digest_sha256: "a".repeat(64),
            version: "1.2.3".into(),
        },
        methods: vec![
            ProviderAuthMethod::AccountBrowser,
            ProviderAuthMethod::DeviceCode,
        ],
        expires_at_unix_seconds: 20,
    }
}

fn unauthenticated_state() -> ProviderAuthState {
    reduce_provider_auth(
        ProviderAuthState::default(),
        ProviderAuthEvent::ObservedUnauthenticated {
            challenge: challenge(),
        },
    )
    .0
}

#[test]
fn unauthenticated_discovery_issues_and_retains_one_typed_challenge() {
    let state = unauthenticated_state();
    let (state, effect) = reduce_provider_auth(
        state,
        ProviderAuthEvent::ObservedUnauthenticated {
            challenge: challenge(),
        },
    );
    assert!(
        matches!(effect, ProviderAuthEffect::AskTool(ref value) if value.challenge_id == "challenge-1")
    );
    assert_eq!(
        state.status.unwrap().lifecycle,
        ProviderAuthLifecycle::ChallengeOffered
    );
}

#[test]
fn exact_offered_selection_starts_login_once() {
    let event = ProviderAuthEvent::SelectMethod {
        selection: ProviderAuthMethodSelection {
            challenge_id: "challenge-1".into(),
            method: ProviderAuthMethod::DeviceCode,
        },
        now: 10,
    };
    let (state, effect) = reduce_provider_auth(unauthenticated_state(), event.clone());
    assert!(matches!(
        effect,
        ProviderAuthEffect::BeginLogin {
            method: ProviderAuthMethod::DeviceCode,
            ..
        }
    ));
    assert_eq!(
        reduce_provider_auth(state, event).1,
        ProviderAuthEffect::None
    );
}

#[test]
fn expiry_cancellation_and_binary_change_are_typed_terminal_states() {
    let (state, effect) = reduce_provider_auth(
        unauthenticated_state(),
        ProviderAuthEvent::Timeout { now: 20 },
    );
    assert!(
        matches!(effect, ProviderAuthEffect::Status(ref value) if value.lifecycle == ProviderAuthLifecycle::Expired)
    );
    assert!(state.challenge.is_none());
    let (state, effect) = reduce_provider_auth(
        unauthenticated_state(),
        ProviderAuthEvent::Cancel {
            challenge_id: "challenge-1".into(),
        },
    );
    assert!(
        matches!(effect, ProviderAuthEffect::Status(ref value) if value.lifecycle == ProviderAuthLifecycle::Cancelled)
    );
    assert!(state.challenge.is_none());
    let (_, effect) = reduce_provider_auth(
        state,
        ProviderAuthEvent::ProviderChanged {
            provider: ProviderAuthProvider::Codex,
            binary_lock: challenge().binary_lock,
        },
    );
    assert!(
        matches!(effect, ProviderAuthEffect::Status(ref value) if value.lifecycle == ProviderAuthLifecycle::ProviderChanged)
    );
}

#[test]
fn terminal_challenge_cannot_be_selected_again() {
    let (state, _) = reduce_provider_auth(
        unauthenticated_state(),
        ProviderAuthEvent::Cancel {
            challenge_id: "challenge-1".into(),
        },
    );
    let (_, effect) = reduce_provider_auth(
        state,
        ProviderAuthEvent::SelectMethod {
            selection: ProviderAuthMethodSelection {
                challenge_id: "challenge-1".into(),
                method: ProviderAuthMethod::AccountBrowser,
            },
            now: 1,
        },
    );
    assert_eq!(
        effect,
        ProviderAuthEffect::Rejected(ProviderAuthRejection::NoActiveChallenge)
    );
}

#[test]
fn invalid_selection_never_starts_login() {
    let (_, effect) = reduce_provider_auth(
        unauthenticated_state(),
        ProviderAuthEvent::SelectMethod {
            selection: ProviderAuthMethodSelection {
                challenge_id: "other".into(),
                method: ProviderAuthMethod::ApiKey,
            },
            now: 1,
        },
    );
    assert_eq!(
        effect,
        ProviderAuthEffect::Rejected(ProviderAuthRejection::ChallengeMismatch)
    );
}
