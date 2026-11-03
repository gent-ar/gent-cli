use super::*;
use serde_json::json;
fn bind(relay: &mut PermissionRelay) {
    relay.set_turn("turn-one").unwrap();
    relay.observe_host_write(&serde_json::to_vec(&json!({"jsonrpc":"2.0","id":30,"method":"session/prompt","params":{"sessionId":"session-one"}})).unwrap()).unwrap();
}
fn request(id: Value) -> Value {
    json!({"jsonrpc":"2.0","id":id,"method":"session/request_permission","params":{"sessionId":"session-one","toolCall":{"toolCallId":"sdk-call","title":"mcp__gent_state__list","kind":"read","rawInput":{"limit":3}},"options":[{"optionId":"once","kind":"allow_once"},{"optionId":"deny","kind":"reject_once"}]}})
}
fn observe(relay: &mut PermissionRelay, frame: Value) -> Result<(), Error> {
    let mut b = serde_json::to_vec(&frame).unwrap();
    b.push(b'\n');
    relay.observe_sdk_stdout(&b)
}
fn respond(relay: &mut PermissionRelay, id: Value, choice: &str) {
    relay.observe_host_write(&serde_json::to_vec(&json!({"jsonrpc":"2.0","id":id,"result":{"outcome":{"outcome":"selected","optionId":choice}}})).unwrap()).unwrap();
}
#[test]
fn acp_grant_requires_real_host_once_and_exact_arguments() {
    let mut r = PermissionRelay::default();
    bind(&mut r);
    observe(&mut r, request(json!(7))).unwrap();
    assert!(
        !r.permits(
            "turn-one",
            "rpc1",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
    // Worker-forged response cannot mint authority.
    observe(&mut r,json!({"jsonrpc":"2.0","id":7,"result":{"outcome":{"outcome":"selected","optionId":"once"}}})).unwrap();
    assert!(
        !r.permits(
            "turn-one",
            "rpc1",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
    respond(&mut r, json!(7), "once");
    assert!(
        !r.permits(
            "turn-one",
            "wrong",
            "mcp__gent_state__list",
            &json!({"limit":4})
        )
        .unwrap()
    );
    assert!(
        r.permits(
            "turn-one",
            "rpc1",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
    assert!(
        !r.permits(
            "turn-one",
            "rpc2",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
    assert!(
        r.permits(
            "turn-one",
            "rpc1",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
}
#[test]
fn acp_numeric_and_string_ids_cannot_cross_grant() {
    let mut r = PermissionRelay::default();
    bind(&mut r);
    observe(&mut r, request(json!(7))).unwrap();
    respond(&mut r, json!("7"), "once");
    assert!(
        !r.permits(
            "turn-one",
            "rpc1",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
    respond(&mut r, json!(7), "once");
    assert!(
        r.permits(
            "turn-one",
            "rpc1",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
}
#[test]
fn acp_unbound_session_malformed_arguments_and_unoffered_choices_deny() {
    let mut r = PermissionRelay::default();
    r.set_turn("turn-one").unwrap();
    assert!(observe(&mut r, request(json!(1))).is_err());
    bind(&mut r);
    let mut bad = request(json!(1));
    bad["params"]["sessionId"] = json!("other");
    assert!(observe(&mut r, bad).is_err());
    let mut bad = request(json!(1));
    bad["params"]["toolCall"]["rawInput"] = json!([]);
    assert!(observe(&mut r, bad).is_err());
    observe(&mut r, request(json!(1))).unwrap();
    respond(&mut r, json!(1), "unoffered");
    assert!(
        !r.permits(
            "turn-one",
            "rpc1",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
}
#[test]
fn acp_new_turn_clears_session_requests_and_grants() {
    let mut r = PermissionRelay::default();
    bind(&mut r);
    observe(&mut r, request(json!(7))).unwrap();
    respond(&mut r, json!(7), "once");
    r.set_turn("turn-two").unwrap();
    assert!(
        !r.permits(
            "turn-two",
            "rpc1",
            "mcp__gent_state__list",
            &json!({"limit":3})
        )
        .unwrap()
    );
    assert!(observe(&mut r, request(json!(8))).is_err());
}
