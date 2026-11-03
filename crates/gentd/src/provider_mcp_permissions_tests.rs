use super::*;
use serde_json::json;
fn request() -> Vec<u8> {
    let mut b=serde_json::to_vec(&json!({"type":"control_request","request_id":"decision","request":{"subtype":"can_use_tool","tool_name":"mcp__server__write","input":{"path":"file"}}})).unwrap();
    b.push(b'\n');
    b
}
fn response(behavior: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"type":"control_response","response":{"subtype":"success","request_id":"decision","response":{"behavior":behavior}}})).unwrap()
}
#[test]
fn mcp_grant_worker_request_and_denial_never_authorize() {
    let mut r = PermissionRelay::default();
    r.set_turn("turn").unwrap();
    r.observe_sdk_stdout(&request()).unwrap();
    assert!(
        !r.permits("turn", "rpc", "mcp__server__write", &json!({"path":"file"}))
            .unwrap()
    );
    r.observe_host_write(&response("deny")).unwrap();
    assert!(
        !r.permits("turn", "rpc", "mcp__server__write", &json!({"path":"file"}))
            .unwrap()
    );
}
#[test]
fn mcp_grant_exact_host_approval_is_one_use_and_replay_bound() {
    let mut r = PermissionRelay::default();
    r.set_turn("turn").unwrap();
    r.observe_sdk_stdout(&request()).unwrap();
    r.observe_host_write(&response("allow")).unwrap();
    let args = json!({"path":"file"});
    assert!(
        !r.permits(
            "turn",
            "rpc",
            "mcp__server__write",
            &json!({"path":"other"})
        )
        .unwrap()
    );
    assert!(
        r.permits("turn", "rpc", "mcp__server__write", &args)
            .unwrap()
    );
    assert!(
        r.permits("turn", "rpc", "mcp__server__write", &args)
            .unwrap()
    );
    assert!(
        !r.permits("turn", "next", "mcp__server__write", &args)
            .unwrap()
    );
    assert!(
        !r.permits(
            "turn",
            "rpc",
            "mcp__server__write",
            &json!({"path":"other"})
        )
        .unwrap()
    );
}
#[test]
fn mcp_grant_updated_input_and_turn_fence_are_exact() {
    let mut r = PermissionRelay::default();
    r.set_turn("turn").unwrap();
    r.observe_sdk_stdout(&request()).unwrap();
    let mut v: Value = serde_json::from_slice(&response("allow")).unwrap();
    v["response"]["response"]["updatedInput"] = json!({"path":"edited"});
    r.observe_host_write(&serde_json::to_vec(&v).unwrap())
        .unwrap();
    assert!(
        !r.permits("turn", "rpc", "mcp__server__write", &json!({"path":"file"}))
            .unwrap()
    );
    assert!(
        r.permits(
            "turn",
            "rpc",
            "mcp__server__write",
            &json!({"path":"edited"})
        )
        .unwrap()
    );
    r.set_turn("next-turn").unwrap();
    assert!(
        !r.permits(
            "next-turn",
            "rpc",
            "mcp__server__write",
            &json!({"path":"edited"})
        )
        .unwrap()
    );
}
#[test]
fn mcp_grant_unsolicited_host_frame_and_partial_frames_do_not_forge() {
    let mut r = PermissionRelay::default();
    r.set_turn("turn").unwrap();
    r.observe_host_write(&response("allow")).unwrap();
    let frame = request();
    let split = frame.len() / 2;
    r.observe_sdk_stdout(&frame[..split]).unwrap();
    r.observe_host_write(&response("allow")).unwrap();
    assert!(
        !r.permits("turn", "rpc", "mcp__server__write", &json!({"path":"file"}))
            .unwrap()
    );
    r.observe_sdk_stdout(&frame[split..]).unwrap();
    r.observe_host_write(&response("allow")).unwrap();
    assert!(
        r.permits("turn", "rpc", "mcp__server__write", &json!({"path":"file"}))
            .unwrap()
    );
}
