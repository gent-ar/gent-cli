use gent_drivers::codex_control::{CodexControlDecision, encode, parse};
use serde_json::{Value, json};

#[test]
fn a_codex_mcp_tool_approval_names_the_exact_tool_and_accepts_without_answers() {
    let request = parse(&json!({"method":"mcpServer/elicitation/request","id":0,"params":{"threadId":"t","turnId":"u","serverName":"gent-goal","mode":"form","_meta":{"codex_approval_kind":"mcp_tool_call","persist":["session","always"],"tool_description":"Update goal status","tool_params":{"status":"complete"}},"message":"Allow the gent-goal MCP server to run tool \"gent_goal_update\"?","requestedSchema":{"type":"object","properties":{}}}}))
        .unwrap()
        .unwrap();
    assert_eq!(request.tool_name, "mcp__gent-goal__gent_goal_update");
    let encoded: Value =
        serde_json::from_slice(&encode(&request, CodexControlDecision::Allow, None)).unwrap();
    assert_eq!(encoded["result"]["action"], "accept");
    assert_eq!(encoded["result"]["content"], json!({}));
}

#[test]
fn structured_mcp_tool_identity_takes_precedence_and_unrecognized_messages_keep_server_scope() {
    let mut frame = json!({"method":"mcpServer/elicitation/request","id":0,"params":{
        "serverName":"gent-chat", "mode":"form", "_meta":{"codex_approval_kind":"mcp_tool_call"},
        "message":"Allow the other MCP server to run tool \"gent_chat_create\"?"}});
    assert_eq!(parse(&frame).unwrap().unwrap().tool_name, "mcp__gent-chat");
    frame["params"]["_meta"]["tool_name"] = json!("gent_chat_create");
    assert_eq!(
        parse(&frame).unwrap().unwrap().tool_name,
        "mcp__gent-chat__gent_chat_create"
    );
    frame["params"]["serverName"] = json!("external");
    assert_eq!(parse(&frame).unwrap().unwrap().tool_name, "mcp__external");
}

#[test]
fn an_ordinary_codex_elicitation_remains_a_user_question() {
    let request = parse(&json!({"method":"mcpServer/elicitation/request","id":1,"params":{"serverName":"gent-goal","mode":"form","message":"Pick one","requestedSchema":{"type":"object","properties":{}}}}))
        .unwrap()
        .unwrap();
    assert_eq!(request.tool_name, "AskUserQuestion");
}
