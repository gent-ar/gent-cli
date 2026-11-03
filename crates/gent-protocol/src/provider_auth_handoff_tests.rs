use super::*;

#[test]
fn versioned_device_method_cannot_enter_legacy_selection() {
    let legacy = serde_json::json!({"type":"selectMethod","body":{"requestId":"select","selection":{"challengeId":"auth-1","method":"deviceVerification"}}});
    assert!(serde_json::from_value::<ProviderAuthFrame>(legacy).is_err());
    let extension = ProviderAuthFrame::HandoffSelectMethod {
        request_id: "select".into(),
        challenge_id: "auth-1".into(),
        method: ProviderAuthHandoffMethod::DeviceVerification,
    };
    extension.validate().unwrap();
    assert_eq!(
        serde_json::from_value::<ProviderAuthFrame>(serde_json::to_value(&extension).unwrap())
            .unwrap(),
        extension
    );
}

#[test]
fn handoff_response_does_not_accept_client_secret_fields() {
    let request = serde_json::json!({"type":"handoffSelectMethod","body":{"requestId":"select","challengeId":"auth-1","method":"deviceVerification","deviceCode":"private"}});
    assert!(serde_json::from_value::<ProviderAuthFrame>(request).is_err());
}
