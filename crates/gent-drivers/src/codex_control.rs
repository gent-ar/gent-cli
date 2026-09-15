use serde_json::{Value, json};

mod answers;
mod redaction;

use answers::{elicitation_content, question_answers};
use redaction::{
    dynamic_tool_name, redacted_approval, redacted_dynamic_tool, redacted_elicitation,
    redacted_questions,
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CodexControlRequest {
    pub request_id: Value,
    pub request_key: String,
    pub method: String,
    pub tool_use_id: String,
    pub tool_name: String,
    pub input: Option<Value>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CodexControlDecision {
    Allow,
    Deny,
}

pub fn parse(frame: &Value) -> Result<Option<CodexControlRequest>, &'static str> {
    let Some(method) = frame.get("method").and_then(Value::as_str) else {
        return Ok(None);
    };
    if !matches!(
        method,
        "item/commandExecution/requestApproval"
            | "item/fileChange/requestApproval"
            | "item/permissions/requestApproval"
            | "item/tool/requestUserInput"
            | "mcpServer/elicitation/request"
            | "item/tool/call"
            | "applyPatchApproval"
            | "execCommandApproval"
    ) {
        return Ok(None);
    }
    let request_id = frame
        .get("id")
        .filter(|value| valid_id(value))
        .cloned()
        .ok_or("malformedCodexControlRequest")?;
    let params = frame
        .get("params")
        .and_then(Value::as_object)
        .ok_or("malformedCodexControlRequest")?;
    let tool_use_id = params
        .get("itemId")
        .and_then(Value::as_str)
        .or_else(|| params.get("callId").and_then(Value::as_str))
        .unwrap_or(method)
        .to_owned();
    let mcp_approval_server = (method == "mcpServer/elicitation/request"
        && params
            .get("_meta")
            .and_then(|meta| meta.get("codex_approval_kind"))
            == Some(&Value::String("mcp_tool_call".into())))
    .then(|| params.get("serverName").and_then(Value::as_str))
    .flatten();
    let tool_name = if method == "item/tool/call" {
        dynamic_tool_name(params)
    } else if let Some(server) = mcp_approval_server {
        format!("mcp__{server}")
    } else {
        match method {
            "item/commandExecution/requestApproval" => "Command",
            "item/fileChange/requestApproval" => "Edit",
            "item/permissions/requestApproval" => "Permission",
            "item/tool/requestUserInput" => "AskUserQuestion",
            "mcpServer/elicitation/request"
                if params.get("mode").and_then(Value::as_str) == Some("url") =>
            {
                "OpenURL"
            }
            "mcpServer/elicitation/request" => "AskUserQuestion",
            "applyPatchApproval" => "Edit",
            "execCommandApproval" => "Bash",
            _ => unreachable!(),
        }
        .to_owned()
    };
    let input = match method {
        "item/tool/requestUserInput" => redacted_questions(params),
        "mcpServer/elicitation/request" => redacted_elicitation(params),
        "item/commandExecution/requestApproval"
        | "item/fileChange/requestApproval"
        | "item/permissions/requestApproval"
        | "applyPatchApproval"
        | "execCommandApproval" => redacted_approval(params, method),
        "item/tool/call" => redacted_dynamic_tool(params),
        _ => None,
    };
    Ok(Some(CodexControlRequest {
        request_key: id_key(&request_id),
        request_id,
        method: method.into(),
        tool_use_id,
        tool_name,
        input,
    }))
}

fn bounded(value: Value) -> Option<Value> {
    serde_json::to_vec(&value)
        .ok()
        .filter(|bytes| bytes.len() <= 16 * 1024)
        .map(|_| value)
}

fn id_key(id: &Value) -> String {
    match id {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        _ => unreachable!("validated JSON-RPC id"),
    }
}

pub fn encode(
    request: &CodexControlRequest,
    decision: CodexControlDecision,
    answers: Option<Value>,
) -> Vec<u8> {
    let allow = decision == CodexControlDecision::Allow;
    let result = match request.method.as_str() {
        "item/tool/requestUserInput" => json!({"answers": question_answers(request, answers)}),
        "mcpServer/elicitation/request" => {
            json!({"action": if allow { "accept" } else { "decline" }, "content": if allow { elicitation_content(request, answers) } else { Value::Null }, "_meta": Value::Null})
        }
        "item/permissions/requestApproval" => {
            let permissions = answers
                .as_ref()
                .and_then(|value| value.get("permissions"))
                .cloned()
                .unwrap_or_else(|| answers.clone().unwrap_or_else(|| json!({})));
            let scope = answers
                .as_ref()
                .and_then(|value| value.get("scope"))
                .and_then(Value::as_str)
                .unwrap_or("turn");
            let mut response =
                json!({"permissions": if allow { permissions } else { json!({}) }, "scope": scope});
            if let Some(strict) = answers
                .as_ref()
                .and_then(|value| value.get("strictAutoReview"))
                .and_then(Value::as_bool)
            {
                response["strictAutoReview"] = Value::Bool(strict);
            }
            response
        }
        "item/tool/call" => json!({
            "success": false,
            "contentItems": answers
                .as_ref()
                .and_then(|value| value.get("contentItems"))
                .filter(|value| value.is_array())
                .cloned()
                .unwrap_or_else(|| json!([{
                "type": "inputText",
                "text": if allow {
                    "Gent approved this dynamic Codex tool call, but no client executor is available for it."
                } else {
                    "User denied this dynamic Codex tool call."
                }
            }]))
        }),
        "applyPatchApproval" | "execCommandApproval" => {
            json!({"decision": if allow { "approved" } else { "denied" }})
        }
        _ => json!({"decision": answers
            .as_ref()
            .and_then(|value| value.get("codexApprovalDecision"))
            .and_then(Value::as_str)
            .unwrap_or(if allow { "accept" } else { "decline" })}),
    };
    let Ok(mut encoded) =
        serde_json::to_vec(&json!({"jsonrpc":"2.0", "id": request.request_id, "result": result}))
    else {
        return Vec::new();
    };
    encoded.push(b'\n');
    encoded
}

fn valid_id(value: &Value) -> bool {
    matches!(value, Value::String(id) if !id.is_empty())
        || matches!(value, Value::Number(id) if id.as_u64().is_some())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn approval_and_elicitation_encode_native_shapes() {
        let request = parse(&json!({"id":1,"method":"item/commandExecution/requestApproval","params":{"itemId":"item"}})).unwrap().unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&encode(&request, CodexControlDecision::Allow, None))
                .unwrap()["result"]["decision"],
            "accept"
        );
        let request =
            parse(&json!({"id":"a","method":"mcpServer/elicitation/request","params":{}}))
                .unwrap()
                .unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&encode(&request, CodexControlDecision::Deny, None))
                .unwrap()["result"]["action"],
            "decline"
        );
    }

    #[test]
    fn extended_native_controls_are_closed_and_encode_their_contracts() {
        for (method, tool_name) in [
            ("applyPatchApproval", "Edit"),
            ("execCommandApproval", "Bash"),
        ] {
            let request =
                parse(&json!({"id":"request","method":method,"params":{"callId":"call"}}))
                    .unwrap()
                    .unwrap();
            assert_eq!(request.tool_name, tool_name);
            assert_eq!(request.tool_use_id, "call");
            let encoded = serde_json::from_slice::<Value>(&encode(
                &request,
                CodexControlDecision::Allow,
                None,
            ))
            .unwrap();
            if matches!(method, "applyPatchApproval" | "execCommandApproval") {
                assert_eq!(encoded["result"]["decision"], "approved");
            }
        }
    }

    #[test]
    fn extended_native_controls_reject_missing_json_rpc_ids_or_params() {
        for method in ["applyPatchApproval", "execCommandApproval"] {
            assert!(parse(&json!({"method":method,"params":{}})).is_err());
            assert!(parse(&json!({"id":1,"method":method})).is_err());
        }
    }

    #[test]
    fn question_and_elicitation_payloads_are_redacted_and_bounded() {
        let question = parse(&json!({
            "id": "q",
            "method": "item/tool/requestUserInput",
            "params": {
            "questions": [{"id":"q1","question":"Choose","options":[{"label":"A"}],"multiSelect":true,"secret":"drop"}],
                "itemId":"item"
            }
        }))
        .unwrap()
        .unwrap();
        assert_eq!(question.input.as_ref().unwrap()["kind"], "questions");
        assert!(
            question.input.as_ref().unwrap()["questions"][0]
                .get("secret")
                .is_none()
        );
        assert_eq!(
            question.input.as_ref().unwrap()["questions"][0]["multiSelect"],
            true
        );

        let elicitation = parse(&json!({
            "id": "e",
            "method": "mcpServer/elicitation/request",
            "params": {"message":"Choose","requestedSchema":{"type":"object","properties":{"name":{"title":"Name","type":"string"}}},"_meta":{"token":"drop"}}
        }))
        .unwrap()
        .unwrap();
        assert_eq!(elicitation.tool_name, "AskUserQuestion");
        assert_eq!(elicitation.input.as_ref().unwrap()["kind"], "questions");
        assert_eq!(
            elicitation.input.as_ref().unwrap()["questions"][0]["id"],
            "name"
        );
        assert!(elicitation.input.as_ref().unwrap().get("_meta").is_none());
    }

    #[test]
    fn answers_are_encoded_for_question_and_elicitation_controls() {
        let question = parse(&json!({"id":"q","method":"item/tool/requestUserInput","params":{}}))
            .unwrap()
            .unwrap();
        let encoded = serde_json::from_slice::<Value>(&encode(
            &question,
            CodexControlDecision::Allow,
            Some(json!({"q1":"A"})),
        ))
        .unwrap();
        assert_eq!(encoded["result"]["answers"]["q1"]["answers"][0], "A");

        let elicitation =
            parse(&json!({"id":"e","method":"mcpServer/elicitation/request","params":{}}))
                .unwrap()
                .unwrap();
        let encoded = serde_json::from_slice::<Value>(&encode(
            &elicitation,
            CodexControlDecision::Allow,
            Some(json!({"name":"A"})),
        ))
        .unwrap();
        assert_eq!(encoded["result"]["content"]["name"], "A");
    }

    #[test]
    fn rich_control_answers_match_codex_wire_shapes() {
        let question = parse(&json!({
            "id": "q",
            "method": "item/tool/requestUserInput",
            "params": {"questions": [{"id": "native-q", "question": "Choose"}]}
        }))
        .unwrap()
        .unwrap();
        let encoded = serde_json::from_slice::<Value>(&encode(
            &question,
            CodexControlDecision::Allow,
            Some(json!({"Choose": ["A", "B"]})),
        ))
        .unwrap();
        assert_eq!(
            encoded["result"]["answers"]["native-q"]["answers"],
            json!(["A", "B"])
        );

        let approval = parse(&json!({
            "id": "a",
            "method": "item/commandExecution/requestApproval",
            "params": {"itemId": "tool", "command": "echo hi", "reason": "run"}
        }))
        .unwrap()
        .unwrap();
        assert_eq!(approval.input.as_ref().unwrap()["command"], "echo hi");
        let encoded = serde_json::from_slice::<Value>(&encode(
            &approval,
            CodexControlDecision::Allow,
            Some(json!({"codexApprovalDecision": "acceptForSession"})),
        ))
        .unwrap();
        assert_eq!(encoded["result"]["decision"], "acceptForSession");

        let elicitation = parse(&json!({
            "id": "e",
            "method": "mcpServer/elicitation/request",
            "params": {
                "message": "Choose",
                "requestedSchema": {"type":"object","properties":{"enabled":{"title":"Enabled","type":"boolean"}}}
            }
        }))
        .unwrap()
        .unwrap();
        let encoded = serde_json::from_slice::<Value>(&encode(
            &elicitation,
            CodexControlDecision::Allow,
            Some(json!({"Enabled": "True"})),
        ))
        .unwrap();
        assert_eq!(encoded["result"]["content"]["enabled"], true);
    }

    #[test]
    fn url_elicitation_is_classified_as_open_url_with_bounded_context() {
        let request = parse(&json!({
            "id": "url",
            "method": "mcpServer/elicitation/request",
            "params": {
                "mode": "url",
                "url": "https://example.test/login",
                "serverName": "docs",
                "elicitationId": "el-1",
                "_meta": {"secret": "drop"}
            }
        }))
        .unwrap()
        .unwrap();
        assert_eq!(request.tool_name, "OpenURL");
        assert_eq!(request.input.as_ref().unwrap()["kind"], "url");
        assert_eq!(
            request.input.as_ref().unwrap()["url"],
            "https://example.test/login"
        );
        assert!(request.input.as_ref().unwrap().get("_meta").is_none());
    }

    #[test]
    fn dynamic_tool_calls_are_relayed_as_provider_controls() {
        let request = parse(&json!({
            "id": "dynamic",
            "method": "item/tool/call",
            "params": {
                "callId": "call-1",
                "namespace": "gent",
                "tool": "lookup",
                "arguments": {"query": "value"}
            }
        }))
        .unwrap()
        .unwrap();
        assert_eq!(request.tool_use_id, "call-1");
        assert_eq!(request.tool_name, "gent:lookup");
        assert_eq!(
            request.input.as_ref().unwrap()["arguments"]["query"],
            "value"
        );
        let response =
            serde_json::from_slice::<Value>(&encode(&request, CodexControlDecision::Allow, None))
                .unwrap();
        assert_eq!(response["result"]["success"], false);
        assert_eq!(response["result"]["contentItems"][0]["type"], "inputText");
    }

    #[test]
    fn oversized_redacted_payloads_are_not_published() {
        let oversized = "x".repeat(17 * 1024);
        let request = parse(&json!({
            "id": "q",
            "method": "item/tool/requestUserInput",
            "params": {"questions": [{"id":"q1","question":oversized}]}
        }))
        .unwrap()
        .unwrap();
        assert!(request.input.is_none());
    }
}
