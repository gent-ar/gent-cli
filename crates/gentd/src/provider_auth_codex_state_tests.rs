use super::CodexAuthState;
use serde_json::json;

fn ready() -> CodexAuthState {
    let mut state = CodexAuthState::new();
    let frames = state
        .receive(&json!({"id":1,"result":{"userAgent":"codex"}}), 100)
        .unwrap();
    assert_eq!(frames[1]["method"], "account/login/start");
    assert_eq!(frames[1]["params"], json!({"type":"chatgptDeviceCode"}));
    state
}

fn offered() -> serde_json::Value {
    json!({"id":2,"result":{"type":"chatgptDeviceCode","loginId":"native-login",
        "verificationUrl":"https://auth.openai.com/codex/device","userCode":"ABCD-EFGH"}})
}

#[test]
fn native_device_offer_projects_only_public_fields_and_cancels_exact_login() {
    let mut state = ready();
    let mut offer = offered();
    offer["result"]["deviceSecret"] = json!("private-native-secret");
    state.receive(&offer, 100).unwrap();
    let public = serde_json::to_string(&state.handoff).unwrap();
    assert!(public.contains("ABCD-EFGH"));
    assert!(!public.contains("native-login"));
    assert!(!public.contains("private-native-secret"));
    assert_eq!(
        state.cancel().unwrap()["params"],
        json!({"loginId":"native-login"})
    );
    let debug = format!("{:?}", state.handoff);
    assert!(!debug.contains("ABCD-EFGH"));
    assert!(!debug.contains("https://"));
}

#[test]
fn completion_requires_exact_native_login_and_clears_public_code() {
    let mut state = ready();
    state.receive(&offered(), 100).unwrap();
    state.receive(&json!({"method":"account/login/completed","params":{"loginId":"wrong","success":true}}),100).unwrap();
    assert_eq!(state.completed, None);
    state.receive(&json!({"method":"account/login/completed","params":{"loginId":"native-login","success":true}}),100).unwrap();
    assert_eq!(state.completed, Some(true));
    assert!(state.handoff.is_none());
}

#[test]
fn matching_completion_before_start_response_is_not_lost_or_replaced_by_other_login() {
    let mut state = ready();
    for id in ["native-login", "different-login"] {
        state
            .receive(
                &json!({"method":"account/login/completed","params":{"loginId":id,"success":true}}),
                100,
            )
            .unwrap();
    }
    state.receive(&offered(), 100).unwrap();
    assert_eq!(state.completed, Some(true));
    assert!(state.handoff.is_none());
}

#[test]
fn unsupported_native_route_or_owner_loopback_is_never_exposed_as_remote_handoff() {
    let mut state = ready();
    assert!(
        state
            .receive(
                &json!({"id":2,"error":{"code":-32602,"message":"unsupported-private-value"}}),
                100
            )
            .is_err()
    );
    assert!(state.handoff.is_none());
    let mut state = ready();
    let mut offer = offered();
    offer["result"]["verificationUrl"] = json!("http://127.0.0.1:1234/callback?code=private");
    assert!(state.receive(&offer, 100).is_err());
    assert!(state.handoff.is_none());
}

#[test]
fn native_client_token_refresh_requests_never_receive_credentials() {
    let mut state = ready();
    let reply = state.receive(&json!({"id":9,"method":"account/chatgptAuthTokens/refresh","params":{"previousAccountId":"private"}}),100).unwrap();
    assert_eq!(reply[0]["error"]["code"], -32601);
    assert!(reply[0].get("result").is_none());
    assert!(!reply[0].to_string().contains("private"));
}
