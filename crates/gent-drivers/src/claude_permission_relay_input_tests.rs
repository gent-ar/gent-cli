use super::*;
use serde_json::json;
fn request(id: &str, tool: &str, input: Value) -> Value {
    json!({"type":"control_request","request_id":id,"request":{"subtype":"can_use_tool","tool_name":"Read","tool_use_id":tool,"input":input,"permission_suggestions":[]}})
}
fn body(bytes: Vec<u8>) -> Value {
    serde_json::from_slice::<Value>(&bytes).unwrap()["response"]["response"].clone()
}
fn allow(relay: &ClaudePermissionRelay, id: &str) -> Value {
    body(
        relay
            .response_with_input(id, ClaudePermissionBehavior::Allow, false, None)
            .unwrap()
            .unwrap(),
    )
}
#[test]
fn original_input_stays_with_its_pending_operation() {
    let mut relay = ClaudePermissionRelay::default();
    relay
        .accept(&request("request-a", "tool-a", json!({"path":"a"})))
        .unwrap();
    relay
        .accept(&request("request-b", "tool-b", json!({"path":"b"})))
        .unwrap();
    assert_eq!(
        allow(&relay, "request-b")["updatedInput"],
        json!({"path":"b"})
    );
    assert_eq!(
        allow(&relay, "request-a")["updatedInput"],
        json!({"path":"a"})
    );
    assert_eq!(
        relay
            .accept(&request("request-a", "other-tool", json!({"path":"other"})))
            .unwrap_err(),
        "duplicateClaudePermissionRequest"
    );
    assert_eq!(
        allow(&relay, "request-a")["updatedInput"],
        json!({"path":"a"})
    );
}
#[test]
fn explicit_reviewed_input_overrides_only_that_operation_and_invalid_update_is_actionable() {
    let mut relay = ClaudePermissionRelay::default();
    relay
        .accept(&request("a", "tool-a", json!({"path":"original-a"})))
        .unwrap();
    relay
        .accept(&request("b", "tool-b", json!({"path":"original-b"})))
        .unwrap();
    assert_eq!(
        body(
            relay
                .response_with_input(
                    "a",
                    ClaudePermissionBehavior::Allow,
                    false,
                    Some(&json!({"path":"reviewed"}))
                )
                .unwrap()
                .unwrap()
        )["updatedInput"],
        json!({"path":"reviewed"})
    );
    assert_eq!(
        allow(&relay, "b")["updatedInput"],
        json!({"path":"original-b"})
    );
    for invalid in [json!("invalid"), json!({"text":"x".repeat(17*1024)})] {
        let error = relay
            .response_with_input("a", ClaudePermissionBehavior::Allow, false, Some(&invalid))
            .unwrap_err();
        assert!(error.contains("16384"));
        assert!(error.contains("remains pending"));
    }
    assert_eq!(
        allow(&relay, "a")["updatedInput"],
        json!({"path":"original-a"})
    );
}
#[test]
fn denial_cancel_and_settlement_never_echo_or_reuse_retained_input() {
    let mut relay = ClaudePermissionRelay::default();
    relay
        .accept(&request(
            "a",
            "tool-a",
            json!({"publicCanary":"private-only-test-canary"}),
        ))
        .unwrap();
    let denied = body(
        relay
            .response_with_input(
                "a",
                ClaudePermissionBehavior::Deny,
                true,
                Some(&json!({"must":"not-echo"})),
            )
            .unwrap()
            .unwrap(),
    );
    assert!(denied.get("updatedInput").is_none());
    assert!(denied.get("updatedPermissions").is_none());
    assert!(!format!("{relay:?}").contains("private-only-test-canary"));
    assert!(relay.cancel(&json!({"request_id":"a"})));
    assert_eq!(relay.retained_bytes, 0);
    assert!(
        relay
            .response_with_input("a", ClaudePermissionBehavior::Allow, false, None)
            .unwrap()
            .is_none()
    );
    relay
        .accept(&request("b", "tool-b", json!({"path":"b"})))
        .unwrap();
    relay.settle("b");
    assert_eq!(relay.retained_bytes, 0);
    assert!(
        relay
            .response_with_input("b", ClaudePermissionBehavior::Allow, false, None)
            .unwrap()
            .is_none()
    );
}
#[test]
fn large_default_input_is_never_rejected_by_retention_budget() {
    let mut relay = ClaudePermissionRelay::default();
    let input = json!({"content":"x".repeat(32*1024)});
    relay
        .accept(&request("a", "tool-a", input.clone()))
        .unwrap();
    assert_eq!(allow(&relay, "a")["updatedInput"], input);
    relay
        .accept(&request(
            "large",
            "tool-large",
            json!({"content":"x".repeat(MAX_PRESERVED_INPUT_BYTES)}),
        ))
        .unwrap();
    let response = allow(&relay, "large");
    assert_eq!(response["behavior"], "allow");
    assert!(response.get("updatedInput").is_none());
    assert_eq!(allow(&relay, "a")["updatedInput"], input);
}
#[test]
fn aggregate_retention_is_soft_and_releases_only_the_exact_pending_capture() {
    let mut relay = ClaudePermissionRelay::default();
    let input = json!({"content":"x".repeat(128*1024)});
    for n in 0..40 {
        relay
            .accept(&request(
                &format!("request-{n}"),
                &format!("tool-{n}"),
                input.clone(),
            ))
            .unwrap();
        assert_eq!(allow(&relay, &format!("request-{n}"))["behavior"], "allow");
    }
    assert!(relay.retained_bytes <= MAX_RETAINED_ORIGINAL_BYTES);
    assert!(relay.pending.values().any(|p| p.original_input.is_none()));
    let before = relay.retained_bytes;
    relay.settle("request-0");
    assert!(relay.retained_bytes < before);
    relay
        .accept(&request("replacement", "replacement", input.clone()))
        .unwrap();
    assert_eq!(allow(&relay, "replacement")["updatedInput"], input);
}
#[test]
fn combined_reply_over_frame_budget_uses_legacy_reply_without_losing_capture() {
    let mut relay = ClaudePermissionRelay::default();
    let input = json!({"content":"x".repeat(400*1024)});
    let mut frame = request("a", "tool-a", input.clone());
    let suggestions = json!([{"type":"addRules","rules":[{"toolName":"Read","ruleContent":"p".repeat(700*1024)}],"behavior":"allow","destination":"session"}]);
    frame["request"]["permission_suggestions"] = suggestions.clone();
    relay.accept(&frame).unwrap();
    let bytes = relay
        .response_with_input("a", ClaudePermissionBehavior::Allow, true, None)
        .unwrap()
        .unwrap();
    assert!(bytes.len() < crate::claude_control::MAX_PERMISSION_REPLY_BYTES);
    let response = body(bytes);
    assert!(response.get("updatedInput").is_none());
    assert_eq!(response["updatedPermissions"], suggestions);
    // The same operation can still echo its original when suggestions are not relayed.
    assert_eq!(allow(&relay, "a")["updatedInput"], input);
}
