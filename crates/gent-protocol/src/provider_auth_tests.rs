use super::{
    MAX_PROVIDER_AUTH_FRAME_BYTES, PROVIDER_AUTH_CAPABILITY, ProviderAuthFrame,
    ProviderAuthFrameError,
};
use gent_types::{
    ProviderAuthBinaryLock, ProviderAuthChallenge, ProviderAuthLifecycle, ProviderAuthMethod,
    ProviderAuthProvider, ProviderAuthStatus,
};
use serde_json::{Value, json};

fn lock() -> ProviderAuthBinaryLock {
    ProviderAuthBinaryLock {
        canonical_executable_id: "provider-id:claude".into(),
        digest_sha256: "a".repeat(64),
        version: "1.2.3".into(),
    }
}

#[test]
fn ask_tool_is_typed_and_bounded() {
    let frame = ProviderAuthFrame::AskTool {
        request_id: "request-1".into(),
        challenge: ProviderAuthChallenge {
            challenge_id: "challenge-1".into(),
            provider: ProviderAuthProvider::Claude,
            binary_lock: lock(),
            methods: vec![ProviderAuthMethod::AccountBrowser],
            expires_at_unix_seconds: 42,
        },
    };
    let encoded = serde_json::to_vec(&frame).unwrap();
    assert!(encoded.len() <= MAX_PROVIDER_AUTH_FRAME_BYTES);
    assert_eq!(frame.validate(), Ok(()));
    assert_eq!(PROVIDER_AUTH_CAPABILITY, "provider-auth-v1");
}

#[test]
fn unknown_fields_are_rejected() {
    let frame = json!({
        "type": "selectMethod", "body": {
            "requestId": "request-1", "selection": {
                "challengeId": "challenge-1", "method": "accountBrowser", "apiKey": "never"
            }
        }
    });
    assert!(serde_json::from_value::<ProviderAuthFrame>(frame).is_err());
}

#[test]
fn client_requests_cannot_supply_a_binary_lock_and_login_is_explicit() {
    let frame = json!({
        "type": "statusRequest", "body": {
            "requestId": "request-1", "provider": "claude", "binaryLock": {}
        }
    });
    assert!(serde_json::from_value::<ProviderAuthFrame>(frame).is_err());
    assert_eq!(
        ProviderAuthFrame::LoginRequest {
            request_id: "request-1".into(),
            provider: ProviderAuthProvider::Claude,
        }
        .validate(),
        Ok(())
    );
}

#[test]
fn serialized_status_has_no_secret_bearing_property_names() {
    let frame = ProviderAuthFrame::Status {
        request_id: "request-1".into(),
        status: ProviderAuthStatus {
            challenge_id: None,
            provider: ProviderAuthProvider::Codex,
            binary_lock: Some(lock()),
            lifecycle: ProviderAuthLifecycle::Authenticated,
            selected_method: Some(ProviderAuthMethod::AccountBrowser),
            expires_at_unix_seconds: Some(42),
        },
    };
    assert_no_secret_keys(&serde_json::to_value(frame).unwrap());
}

#[test]
fn lifecycle_vocabulary_is_closed_and_secret_free() {
    let not_installed = ProviderAuthFrame::Status {
        request_id: "request-1".into(),
        status: ProviderAuthStatus {
            challenge_id: None,
            provider: ProviderAuthProvider::Claude,
            binary_lock: None,
            lifecycle: ProviderAuthLifecycle::NotInstalled,
            selected_method: None,
            expires_at_unix_seconds: None,
        },
    };
    assert_eq!(
        serde_json::to_value(&not_installed).unwrap(),
        json!({
            "type": "status",
            "body": {
                "requestId": "request-1",
                "status": {
                    "provider": "claude", "binaryLock": null, "lifecycle": "notInstalled",
                    "selectedMethod": null, "expiresAtUnixSeconds": null
                }
            }
        })
    );
    for lifecycle in [
        ProviderAuthLifecycle::NotInstalled,
        ProviderAuthLifecycle::Checking,
        ProviderAuthLifecycle::Unauthenticated,
        ProviderAuthLifecycle::ChallengeOffered,
        ProviderAuthLifecycle::Verifying,
        ProviderAuthLifecycle::Authenticated,
        ProviderAuthLifecycle::Expired,
        ProviderAuthLifecycle::Cancelled,
        ProviderAuthLifecycle::TimedOut,
        ProviderAuthLifecycle::Failed,
        ProviderAuthLifecycle::ProviderChanged,
    ] {
        let frame = ProviderAuthFrame::SelectionAccepted {
            request_id: "request-1".into(),
            status: ProviderAuthStatus {
                challenge_id: None,
                provider: ProviderAuthProvider::Codex,
                binary_lock: Some(lock()),
                lifecycle,
                selected_method: Some(ProviderAuthMethod::AccountBrowser),
                expires_at_unix_seconds: Some(42),
            },
        };
        let encoded = serde_json::to_value(&frame).unwrap();
        assert_no_secret_keys(&encoded);
        assert_eq!(
            serde_json::from_value::<ProviderAuthFrame>(encoded).unwrap(),
            frame
        );
    }
    for retired in ["unknown", "openingBrowser", "awaitingDeviceApproval"] {
        let frame = json!({
            "type": "status", "body": {
                "requestId": "request-1", "status": {
                    "provider": "claude", "binaryLock": null, "lifecycle": retired,
                    "selectedMethod": null, "expiresAtUnixSeconds": null
                }
            }
        });
        assert!(serde_json::from_value::<ProviderAuthFrame>(frame).is_err());
    }
    for method in ["deviceCode", "apiKey", "accessToken"] {
        let frame = json!({
            "type": "selectMethod", "body": {
                "requestId": "request-1",
                "selection": { "challengeId": "challenge-1", "method": method }
            }
        });
        assert!(serde_json::from_value::<ProviderAuthFrame>(frame).is_err());
    }
    let with_device_code = json!({
        "type": "status", "body": {
            "requestId": "request-1", "status": {
                "provider": "claude", "binaryLock": null, "lifecycle": "verifying",
                "selectedMethod": "accountBrowser", "expiresAtUnixSeconds": 1,
                "deviceCode": "never"
            }
        }
    });
    assert!(serde_json::from_value::<ProviderAuthFrame>(with_device_code).is_err());
}

#[test]
fn an_oversized_request_is_rejected_before_transport() {
    let frame = ProviderAuthFrame::Cancel {
        request_id: "r".repeat(129),
        challenge_id: "challenge-1".into(),
    };
    assert_eq!(
        frame.validate(),
        Err(ProviderAuthFrameError::InvalidIdentifier)
    );
}

fn assert_no_secret_keys(value: &Value) {
    const FORBIDDEN: [&str; 9] = [
        "apiKey",
        "accessToken",
        "token",
        "secret",
        "password",
        "credential",
        "deviceCode",
        "callback",
        "stdout",
    ];
    match value {
        Value::Object(map) => {
            for (key, nested) in map {
                assert!(
                    !FORBIDDEN.contains(&key.as_str()),
                    "forbidden property: {key}"
                );
                assert_no_secret_keys(nested);
            }
        }
        Value::Array(items) => items.iter().for_each(assert_no_secret_keys),
        _ => {}
    }
}
