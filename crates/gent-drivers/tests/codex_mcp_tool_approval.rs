use gent_drivers::codex_control::{CodexControlDecision, encode, parse};
use serde_json::{Value, json};

#[test]
fn a_codex_mcp_tool_approval_names_its_server_and_accepts_without_answers() {
    let request = parse(&json!({"method":"mcpServer/elicitation/request","id":0,"params":{"threadId":"t","turnId":"u","serverName":"gent-goal","mode":"form","_meta":{"codex_approval_kind":"mcp_tool_call","persist":["session","always"],"tool_description":"Update goal status","tool_params":{"status":"complete"}},"message":"Allow the gent-goal MCP server to run tool \"gent_goal_update\"?","requestedSchema":{"type":"object","properties":{}}}}))
        .unwrap()
        .unwrap();
    assert_eq!(request.tool_name, "mcp__gent-goal");
    let encoded: Value =
        serde_json::from_slice(&encode(&request, CodexControlDecision::Allow, None)).unwrap();
    assert_eq!(encoded["result"]["action"], "accept");
    assert_eq!(encoded["result"]["content"], json!({}));
}

#[test]
fn an_ordinary_codex_elicitation_remains_a_user_question() {
    let request = parse(&json!({"method":"mcpServer/elicitation/request","id":1,"params":{"serverName":"gent-goal","mode":"form","message":"Pick one","requestedSchema":{"type":"object","properties":{}}}}))
        .unwrap()
        .unwrap();
    assert_eq!(request.tool_name, "AskUserQuestion");
}
