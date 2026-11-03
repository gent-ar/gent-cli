use super::*;
use serde_json::json;
struct Wire {
    writes: Vec<Vec<u8>>,
}
impl ClaurstAcpStdio for Wire {
    fn write_frame(&mut self, frame: &[u8]) -> Result<(), String> {
        self.writes.push(frame.into());
        Ok(())
    }
    fn try_read_frame(&mut self, _: usize) -> Result<Option<Vec<u8>>, String> {
        Ok(None)
    }
}
fn update(id: &str, path: &str) -> Value {
    json!({"method":"session/update","params":{"update":{"sessionUpdate":"tool_call","toolCallId":id,
        "title":"Write","kind":"edit","status":"in_progress","rawInput":{"file_path":path,"content":path}}}})
}
fn permission(rpc_id: u64, id: &str, path: &str) -> Value {
    json!({"id":rpc_id,"method":"session/request_permission","params":{"toolCall":{"toolCallId":id,
        "title":"Write","kind":"edit","rawInput":{"file_path":path,"content":path}},
        "options":[{"kind":"allow_once","optionId":"allow_once"}]}})
}
fn feed(
    t: &mut ClaurstAcpTransport<Wire>,
    frame: Value,
) -> Result<Option<ClaurstAcpTerminal>, ClaurstAcpTransportError> {
    t.handle_frame(serde_json::to_vec(&frame).unwrap())
}
#[test]
fn approval_uses_the_offered_option_and_never_invents_one() {
    let mut t = ClaurstAcpTransport::new(Wire { writes: vec![] });
    let mut first = permission(1, "a", "a.txt");
    first["params"]["options"] = json!([{"kind":"allow_once","optionId":"provider-choice-42"}]);
    feed(&mut t, first).unwrap();
    t.respond_permission("claurst:n:1", ClaurstPermissionReply::AllowOnce)
        .unwrap();
    let reply: Value = serde_json::from_slice(&t.stdio.writes[0]).unwrap();
    assert_eq!(reply["result"]["outcome"]["optionId"], "provider-choice-42");

    let mut second = permission(2, "b", "b.txt");
    second["params"]["options"] = json!([{"kind":"reject_once","optionId":"no"}]);
    feed(&mut t, second).unwrap();
    assert!(
        t.respond_permission("claurst:n:2", ClaurstPermissionReply::AllowOnce)
            .is_err()
    );
    assert_eq!(t.stdio.writes.len(), 1);
    t.respond_permission("claurst:n:2", ClaurstPermissionReply::Deny)
        .unwrap();
    assert_eq!(t.stdio.writes.len(), 2);
}
#[test]
fn reversed_same_kind_acp_requests_keep_preview_and_response_rpc_bound() {
    let mut t = ClaurstAcpTransport::new(Wire { writes: vec![] });
    feed(&mut t, update("a", "a.txt")).unwrap();
    feed(&mut t, update("b", "b.txt")).unwrap();
    feed(&mut t, permission(72, "b", "b.txt")).unwrap();
    feed(&mut t, permission(71, "a", "a.txt")).unwrap();
    let first = t.drain(32).unwrap().permissions;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].tool_use_id, "b");
    assert_eq!(first[0].input.as_ref().unwrap()["file_path"], "b.txt");
    assert!(
        t.respond_permission("claurst:n:71", ClaurstPermissionReply::AllowOnce)
            .is_err()
    );
    assert!(t.stdio.writes.is_empty());
    t.respond_permission("claurst:n:72", ClaurstPermissionReply::Deny)
        .unwrap();
    let b: Value = serde_json::from_slice(&t.stdio.writes[0]).unwrap();
    assert_eq!(b["id"], 72);
    assert_eq!(b["result"]["outcome"]["outcome"], "cancelled");
    let second = t.drain(32).unwrap().permissions;
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].tool_use_id, "a");
    assert_eq!(second[0].input.as_ref().unwrap()["file_path"], "a.txt");
    t.respond_permission("claurst:n:71", ClaurstPermissionReply::AllowOnce)
        .unwrap();
    let a: Value = serde_json::from_slice(&t.stdio.writes[1]).unwrap();
    assert_eq!(a["id"], 71);
    assert_eq!(a["result"]["outcome"]["optionId"], "allow_once");
    assert!(
        t.respond_permission("claurst:n:71", ClaurstPermissionReply::AllowOnce)
            .is_err()
    );
}
#[test]
fn legacy_missing_input_conflict_and_replay_never_present_guessed_permissions() {
    let mut t = ClaurstAcpTransport::new(Wire { writes: vec![] });
    feed(&mut t, update("a", "a.txt")).unwrap();
    let mut legacy = permission(71, "a", "a.txt");
    legacy["params"]["toolCall"]
        .as_object_mut()
        .unwrap()
        .remove("rawInput");
    assert!(feed(&mut t, legacy).is_err());
    assert!(feed(&mut t, permission(72, "a", "different.txt")).is_err());
    assert!(t.drain(32).unwrap().permissions.is_empty());
    assert!(t.stdio.writes.is_empty());
    feed(&mut t, permission(73, "a", "a.txt")).unwrap();
    t.respond_permission("claurst:n:73", ClaurstPermissionReply::Deny)
        .unwrap();
    assert!(feed(&mut t, permission(74, "a", "a.txt")).is_err());
}
#[test]
fn callback_before_ui_update_retains_own_operation_identity() {
    let mut t = ClaurstAcpTransport::new(Wire { writes: vec![] });
    feed(&mut t, update("a", "a.txt")).unwrap();
    feed(&mut t, permission(72, "b", "b.txt")).unwrap();
    let request = t.drain(32).unwrap().permissions.pop().unwrap();
    assert_eq!(request.tool_use_id, "b");
    assert_eq!(request.input.unwrap()["file_path"], "b.txt");
    t.respond_permission("claurst:n:72", ClaurstPermissionReply::Deny)
        .unwrap();
    feed(&mut t, update("b", "b.txt")).unwrap();
    assert!(feed(&mut t, permission(73, "b", "b.txt")).is_err());
}

#[test]
fn duplicate_rpc_id_cannot_release_another_operation() {
    let mut t = ClaurstAcpTransport::new(Wire { writes: vec![] });
    feed(&mut t, permission(71, "a", "a.txt")).unwrap();
    assert!(feed(&mut t, permission(71, "b", "b.txt")).is_err());
    let first = t.drain(32).unwrap().permissions;
    assert_eq!(first.len(), 1);
    assert_eq!(first[0].tool_use_id, "a");
    t.respond_permission("claurst:n:71", ClaurstPermissionReply::Deny)
        .unwrap();
    assert!(t.drain(32).unwrap().permissions.is_empty());
    feed(&mut t, permission(72, "b", "b.txt")).unwrap();
    assert_eq!(t.drain(32).unwrap().permissions[0].tool_use_id, "b");
}

#[test]
fn rpc_wire_identity_is_shared_between_mcp_and_builtin_namespaces() {
    for mcp_first in [false, true] {
        let mut t = ClaurstAcpTransport::new(Wire { writes: vec![] });
        let mut mcp = permission(71, "mcp-op", "unused");
        mcp["params"]["toolCall"]["title"] = "mcp__owned__probe".into();
        mcp["params"]["toolCall"]["rawInput"] = json!({});
        mcp["params"]["toolCall"]["kind"] = "other".into();
        let builtin = permission(71, "builtin-op", "a.txt");
        let (first, mut second) = if mcp_first {
            (mcp, builtin)
        } else {
            (builtin, mcp)
        };
        feed(&mut t, first).unwrap();
        assert!(feed(&mut t, second.clone()).is_err());
        second["id"] = json!(72);
        feed(&mut t, second).unwrap();
        assert_eq!(t.permission_rpc_ids.len(), 2);
        assert_eq!(t.held_permissions.len(), 1);
        let pending = t.pending_permission.as_ref().unwrap().request_id.clone();
        t.respond_permission(&pending, ClaurstPermissionReply::Deny)
            .unwrap();
        let pending = t.pending_permission.as_ref().unwrap().request_id.clone();
        t.respond_permission(&pending, ClaurstPermissionReply::Deny)
            .unwrap();
        let ids: Vec<Value> = t
            .stdio
            .writes
            .iter()
            .map(|frame| serde_json::from_slice::<Value>(frame).unwrap()["id"].clone())
            .collect();
        assert_eq!(ids, vec![json!(71), json!(72)]);
    }
}

#[test]
fn number_and_string_wire_ids_have_distinct_api_ids_and_responses() {
    let mut t = ClaurstAcpTransport::new(Wire { writes: vec![] });
    feed(&mut t, permission(71, "number-op", "number.txt")).unwrap();
    let mut string = permission(71, "string-op", "string.txt");
    string["id"] = json!("71");
    feed(&mut t, string).unwrap();
    assert_eq!(
        t.pending_permission.as_ref().unwrap().request_id,
        "claurst:n:71"
    );
    assert_eq!(t.held_permissions[0].0.request_id, "claurst:s:71");
    assert!(
        t.respond_permission("claurst:s:71", ClaurstPermissionReply::AllowOnce)
            .is_err()
    );
    t.respond_permission("claurst:n:71", ClaurstPermissionReply::Deny)
        .unwrap();
    t.respond_permission("claurst:s:71", ClaurstPermissionReply::AllowOnce)
        .unwrap();
    let numeric: Value = serde_json::from_slice(&t.stdio.writes[0]).unwrap();
    let string: Value = serde_json::from_slice(&t.stdio.writes[1]).unwrap();
    assert_eq!(numeric["id"], json!(71));
    assert_eq!(numeric["result"]["outcome"]["outcome"], "cancelled");
    assert_eq!(string["id"], json!("71"));
    assert_eq!(string["result"]["outcome"]["optionId"], "allow_once");
    feed(&mut t, permission(72, "fresh-op", "fresh.txt")).unwrap();
    assert_eq!(
        t.pending_permission.as_ref().unwrap().request_id,
        "claurst:n:72"
    );
}
#[test]
fn mcp_prefix_looking_string_cannot_alias_an_mcp_numeric_api_id() {
    for mcp_first in [false, true] {
        let mut t = ClaurstAcpTransport::new(Wire { writes: vec![] });
        let mut mcp = permission(71, "mcp-op", "unused");
        mcp["params"]["toolCall"]["title"] = json!("mcp__owned__probe");
        mcp["params"]["toolCall"]["rawInput"] = json!({});
        mcp["params"]["toolCall"]["kind"] = json!("other");
        let mut builtin = permission(71, "builtin-op", "builtin.txt");
        builtin["id"] = json!("mcp:n:71");
        let (first, second) = if mcp_first {
            (mcp, builtin)
        } else {
            (builtin, mcp)
        };
        feed(&mut t, first).unwrap();
        feed(&mut t, second).unwrap();
        let first = t.pending_permission.as_ref().unwrap().request_id.clone();
        let second = t.held_permissions[0].0.request_id.clone();
        let expected_first = if mcp_first {
            "claurst:n:71"
        } else {
            "claurst:s:mcp:n:71"
        };
        let expected_second = if mcp_first {
            "claurst:s:mcp:n:71"
        } else {
            "claurst:n:71"
        };
        assert_eq!(first, expected_first);
        assert_eq!(second, expected_second);
        assert_ne!(first, second);
        assert!(
            t.respond_permission(&second, ClaurstPermissionReply::AllowOnce)
                .is_err()
        );
        t.respond_permission(&first, ClaurstPermissionReply::Deny)
            .unwrap();
        t.respond_permission(&second, ClaurstPermissionReply::Deny)
            .unwrap();
        let response_ids: Vec<Value> = t
            .stdio
            .writes
            .iter()
            .map(|frame| serde_json::from_slice::<Value>(frame).unwrap()["id"].clone())
            .collect();
        assert_eq!(
            response_ids,
            if mcp_first {
                vec![json!(71), json!("mcp:n:71")]
            } else {
                vec![json!("mcp:n:71"), json!(71)]
            }
        );
        feed(&mut t, permission(72, "fresh-op", "fresh.txt")).unwrap();
        assert_eq!(
            t.pending_permission.as_ref().unwrap().request_id,
            "claurst:n:72"
        );
    }
}
