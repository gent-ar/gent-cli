//! Pure, secret-free provider authentication state transitions.

use gent_types::{
    ProviderAuthBinaryLock, ProviderAuthChallenge, ProviderAuthLifecycle, ProviderAuthMethod,
    ProviderAuthMethodSelection, ProviderAuthProvider, ProviderAuthStatus,
};

/// Reducer state for one public provider authentication challenge.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct ProviderAuthState {
    pub challenge: Option<ProviderAuthChallenge>,
    pub status: Option<ProviderAuthStatus>,
}

/// Facts and method-only requests accepted by the authentication reducer.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderAuthEvent {
    ObservedUnauthenticated {
        challenge: ProviderAuthChallenge,
    },
    ObservedAuthenticated {
        provider: ProviderAuthProvider,
        binary_lock: ProviderAuthBinaryLock,
    },
    SelectMethod {
        selection: ProviderAuthMethodSelection,
        now: u64,
    },
    Cancel {
        challenge_id: String,
    },
    Timeout {
        now: u64,
    },
    ProviderChanged {
        provider: ProviderAuthProvider,
        binary_lock: ProviderAuthBinaryLock,
    },
}

/// A future launcher may act only on [`Self::BeginLogin`], never raw client input.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProviderAuthEffect {
    None,
    AskTool(ProviderAuthChallenge),
    BeginLogin {
        provider: ProviderAuthProvider,
        binary_lock: ProviderAuthBinaryLock,
        challenge_id: String,
        method: ProviderAuthMethod,
    },
    Status(ProviderAuthStatus),
    Rejected(ProviderAuthRejection),
}

/// Rejections intentionally exclude raw values and credential-like data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProviderAuthRejection {
    InvalidChallenge,
    NoActiveChallenge,
    ChallengeMismatch,
    MethodNotOffered,
}

/// Reduces one typed provider-auth event without process, persistence, or clock access.
#[must_use]
pub fn reduce_provider_auth(
    mut state: ProviderAuthState,
    event: ProviderAuthEvent,
) -> (ProviderAuthState, ProviderAuthEffect) {
    match event {
        ProviderAuthEvent::ObservedUnauthenticated { challenge } => {
            issue_or_retain(state, challenge)
        }
        ProviderAuthEvent::ObservedAuthenticated {
            provider,
            binary_lock,
        } => {
            let status = status(
                provider,
                binary_lock,
                ProviderAuthLifecycle::Authenticated,
                None,
                None,
            );
            state.challenge = None;
            state.status = Some(status.clone());
            (state, ProviderAuthEffect::Status(status))
        }
        ProviderAuthEvent::SelectMethod { selection, now } => select(state, &selection, now),
        ProviderAuthEvent::Cancel { challenge_id } => cancel(state, &challenge_id),
        ProviderAuthEvent::Timeout { now } => timeout(state, now),
        ProviderAuthEvent::ProviderChanged {
            provider,
            binary_lock,
        } => {
            let status = status(
                provider,
                binary_lock,
                ProviderAuthLifecycle::ProviderChanged,
                None,
                None,
            );
            state.challenge = None;
            state.status = Some(status.clone());
            (state, ProviderAuthEffect::Status(status))
        }
    }
}

fn issue_or_retain(
    mut state: ProviderAuthState,
    challenge: ProviderAuthChallenge,
) -> (ProviderAuthState, ProviderAuthEffect) {
    if challenge.validate().is_err() {
        return (
            state,
            ProviderAuthEffect::Rejected(ProviderAuthRejection::InvalidChallenge),
        );
    }
    let selected = state
        .challenge
        .as_ref()
        .filter(|current| same_binary(current, &challenge))
        .cloned()
        .unwrap_or(challenge);
    let status = status_for_challenge(&selected, ProviderAuthLifecycle::ChallengeOffered, None);
    state.challenge = Some(selected.clone());
    state.status = Some(status);
    (state, ProviderAuthEffect::AskTool(selected))
}

fn select(
    mut state: ProviderAuthState,
    selection: &ProviderAuthMethodSelection,
    now: u64,
) -> (ProviderAuthState, ProviderAuthEffect) {
    let Some(challenge) = state.challenge.clone() else {
        return (
            state,
            ProviderAuthEffect::Rejected(ProviderAuthRejection::NoActiveChallenge),
        );
    };
    if selection.validate().is_err() || selection.challenge_id != challenge.challenge_id {
        return (
            state,
            ProviderAuthEffect::Rejected(ProviderAuthRejection::ChallengeMismatch),
        );
    }
    if challenge.expires_at_unix_seconds <= now {
        return expire(state, &challenge);
    }
    if !challenge.methods.contains(&selection.method) {
        return (
            state,
            ProviderAuthEffect::Rejected(ProviderAuthRejection::MethodNotOffered),
        );
    }
    if state.status.as_ref().is_some_and(|current| {
        current.lifecycle == ProviderAuthLifecycle::Verifying
            && current.selected_method == Some(selection.method)
    }) {
        return (state, ProviderAuthEffect::None);
    }
    let status = status_for_challenge(
        &challenge,
        ProviderAuthLifecycle::Verifying,
        Some(selection.method),
    );
    state.status = Some(status);
    (
        state,
        ProviderAuthEffect::BeginLogin {
            provider: challenge.provider,
            binary_lock: challenge.binary_lock,
            challenge_id: challenge.challenge_id,
            method: selection.method,
        },
    )
}

fn cancel(
    mut state: ProviderAuthState,
    challenge_id: &str,
) -> (ProviderAuthState, ProviderAuthEffect) {
    let Some(challenge) = state.challenge.clone() else {
        return (
            state,
            ProviderAuthEffect::Rejected(ProviderAuthRejection::NoActiveChallenge),
        );
    };
    if challenge.challenge_id != challenge_id {
        return (
            state,
            ProviderAuthEffect::Rejected(ProviderAuthRejection::ChallengeMismatch),
        );
    }
    let status = status_for_challenge(&challenge, ProviderAuthLifecycle::Cancelled, None);
    state.challenge = None;
    state.status = Some(status.clone());
    (state, ProviderAuthEffect::Status(status))
}

fn timeout(state: ProviderAuthState, now: u64) -> (ProviderAuthState, ProviderAuthEffect) {
    let Some(challenge) = state.challenge.clone() else {
        return (state, ProviderAuthEffect::None);
    };
    if challenge.expires_at_unix_seconds > now {
        return (state, ProviderAuthEffect::None);
    }
    expire(state, &challenge)
}

fn expire(
    mut state: ProviderAuthState,
    challenge: &ProviderAuthChallenge,
) -> (ProviderAuthState, ProviderAuthEffect) {
    let status = status_for_challenge(challenge, ProviderAuthLifecycle::Expired, None);
    state.challenge = None;
    state.status = Some(status.clone());
    (state, ProviderAuthEffect::Status(status))
}

fn same_binary(left: &ProviderAuthChallenge, right: &ProviderAuthChallenge) -> bool {
    left.provider == right.provider && left.binary_lock == right.binary_lock
}

fn status_for_challenge(
    challenge: &ProviderAuthChallenge,
    lifecycle: ProviderAuthLifecycle,
    method: Option<ProviderAuthMethod>,
) -> ProviderAuthStatus {
    status(
        challenge.provider,
        challenge.binary_lock.clone(),
        lifecycle,
        method,
        Some(challenge.expires_at_unix_seconds),
    )
}

fn status(
    provider: ProviderAuthProvider,
    binary_lock: ProviderAuthBinaryLock,
    lifecycle: ProviderAuthLifecycle,
    selected_method: Option<ProviderAuthMethod>,
    expires_at_unix_seconds: Option<u64>,
) -> ProviderAuthStatus {
    ProviderAuthStatus {
        provider,
        binary_lock,
        lifecycle,
        selected_method,
        expires_at_unix_seconds,
    }
}

#[cfg(test)]
#[path = "provider_auth_tests.rs"]
mod tests;
