use super::*;
use serde_json::json;
struct Empty;
impl crate::claurst_acp_transport::ClaurstAcpStdio for Empty {
    fn write_frame(&mut self, _: &[u8]) -> Result<(), String> {
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }
}
fn call(id: &str, name: &str, input: Value) -> Value {
    json!({"toolCallId":id,"title":name,"kind":"edit","rawInput":input})
}
fn start(transport: &mut ClaurstAcpTransport<Empty>, id: &str, name: &str, input: Value) {
    transport.session_update_fact(Some(&json!({"update":{
        "sessionUpdate":"tool_call","toolCallId":id,"title":name,
        "kind":"edit","status":"in_progress","rawInput":input
    }})));
}
#[test]
fn reversed_concurrent_write_callbacks_keep_exact_operation_and_input() {
    let mut transport = ClaurstAcpTransport::new(Empty);
    let a = json!({"file_path":"a","content":"a"});
    let b = json!({"file_path":"b","content":"b"});
    start(&mut transport, "a", "Write", a.clone());
    start(&mut transport, "b", "Write", b.clone());
    for (id, input) in [("b", b), ("a", a)] {
        let claim = transport
            .claim_permission_tool(&call(id, "Write", input.clone()))
            .unwrap();
        assert_eq!(claim.tool_use_id, id);
        assert_eq!(claim.tool_name, "Write");
        assert_eq!(claim.input, Some(input));
    }
    assert!(
        transport
            .claim_permission_tool(&call("a", "Write", json!({"file_path":"a","content":"a"})))
            .is_none()
    );
}
#[test]
fn permission_before_display_update_never_claims_another_same_kind_tool() {
    let mut transport = ClaurstAcpTransport::new(Empty);
    start(&mut transport, "a", "Write", json!({"file_path":"a"}));
    let b = call("b", "Write", json!({"file_path":"b"}));
    assert_eq!(
        transport.claim_permission_tool(&b).unwrap().tool_use_id,
        "b"
    );
    assert!(!transport.open_tools[0].permission_claimed);
    assert!(transport.claim_permission_tool(&b).is_none());
}
#[test]
fn missing_input_changed_payload_or_changed_name_cannot_bind() {
    let mut transport = ClaurstAcpTransport::new(Empty);
    start(&mut transport, "a", "Write", json!({"file_path":"a"}));
    assert!(
        transport
            .claim_permission_tool(&json!({"toolCallId":"a","title":"Write","kind":"edit"}))
            .is_none()
    );
    assert!(
        transport
            .claim_permission_tool(&call("a", "Write", json!({"file_path":"b"})))
            .is_none()
    );
    assert!(
        transport
            .claim_permission_tool(&call("a", "Edit", json!({"file_path":"a"})))
            .is_none()
    );
    assert!(
        transport
            .claim_permission_tool(&call("a", "Write", json!({"file_path":"a"})))
            .is_some()
    );
}
#[test]
fn authoritative_long_name_is_not_the_truncated_ui_label() {
    let mut transport = ClaurstAcpTransport::new(Empty);
    let name = format!("mcp__{}", "x".repeat(100));
    start(&mut transport, "long", &name, json!({}));
    assert_eq!(transport.open_tools[0].name.len(), 80);
    assert_eq!(
        transport
            .claim_permission_tool(&call("long", &name, json!({})))
            .unwrap()
            .tool_name,
        name
    );
    let mut early = ClaurstAcpTransport::new(Empty);
    assert_eq!(
        early
            .claim_permission_tool(&call("long", &name, json!({})))
            .unwrap()
            .tool_name,
        name
    );
    start(&mut early, "long", &name, json!({}));
    assert!(
        early
            .claim_permission_tool(&call("long", &name, json!({})))
            .is_none()
    );
}

#[test]
fn documented_mcp_dot_names_are_preserved_exactly() {
    let mut transport = ClaurstAcpTransport::new(Empty);
    let name = "mcp__server.v1__tool.name";
    start(&mut transport, "dot", name, json!({}));
    assert_eq!(
        transport
            .claim_permission_tool(&call("dot", name, json!({})))
            .unwrap()
            .tool_name,
        name
    );
}
