use super::*;

fn state() -> ProviderAuthHandoffState {
    let lock = crate::ProviderAuthBinaryLock {
        canonical_executable_id: "provider-id:codex".into(),
        digest_sha256: "a".repeat(64),
        version: "codex-cli 0.145.0".into(),
    };
    let challenge = ProviderAuthChallenge {
        challenge_id: "challenge-1".into(),
        provider: crate::ProviderAuthProvider::Codex,
        binary_lock: lock.clone(),
        methods: vec![crate::ProviderAuthMethod::AccountBrowser],
        expires_at_unix_seconds: 100,
    };
    ProviderAuthHandoffState {
        status: ProviderAuthStatus {
            challenge_id: Some("challenge-1".into()),
            provider: challenge.provider,
            binary_lock: Some(lock),
            lifecycle: crate::ProviderAuthLifecycle::Verifying,
            selected_method: None,
            expires_at_unix_seconds: Some(100),
        },
        challenge: Some(challenge),
        methods: vec![ProviderAuthHandoffMethod::DeviceVerification],
        handoff: Some(ProviderAuthHandoff::DeviceVerification {
            verification_url: "https://auth.openai.com/codex/device".into(),
            user_code: "ABCD-EFGH".into(),
            expires_at_unix_seconds: 100,
        }),
    }
}

#[test]
fn public_device_view_roundtrips_but_debug_never_exposes_instructions() {
    let state = state();
    assert_eq!(state.validate(), Ok(()));
    let json = serde_json::to_vec(&state).unwrap();
    assert_eq!(
        serde_json::from_slice::<ProviderAuthHandoffState>(&json).unwrap(),
        state
    );
    let debug = format!("{state:?}");
    assert!(!debug.contains("ABCD-EFGH"));
    assert!(!debug.contains("https://"));
}

#[test]
fn mismatched_provider_challenge_binary_and_expiry_fail_closed() {
    for change in 0..5 {
        let mut state = state();
        match change {
            0 => state.status.provider = crate::ProviderAuthProvider::Claude,
            1 => state.status.challenge_id = Some("other".into()),
            2 => state.status.binary_lock.as_mut().unwrap().digest_sha256 = "b".repeat(64),
            3 => state.status.expires_at_unix_seconds = Some(101),
            _ => state.challenge = None,
        }
        assert!(state.validate().is_err());
    }
}

#[test]
fn private_tokens_are_not_a_handoff_field_and_cancelled_status_cannot_retain_code() {
    let mut wire = serde_json::to_value(state()).unwrap();
    wire["handoff"]["accessToken"] = serde_json::json!("private");
    assert!(serde_json::from_value::<ProviderAuthHandoffState>(wire).is_err());
    let mut state = state();
    state.status.lifecycle = crate::ProviderAuthLifecycle::Cancelled;
    assert!(state.validate().is_err());
}
