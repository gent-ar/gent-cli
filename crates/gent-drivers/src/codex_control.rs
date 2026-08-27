use serde_json::{Value, json};

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
    let tool_name = if method == "item/tool/call" {
        dynamic_tool_name(params)
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

fn dynamic_tool_name(params: &serde_json::Map<String, Value>) -> String {
    let tool = params
        .get("tool")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    let namespace = params
        .get("namespace")
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty());
    match (namespace, tool) {
        (Some(namespace), Some(tool)) => format!("{namespace}:{tool}"),
        (None, Some(tool)) => tool.to_owned(),
        _ => "DynamicTool".to_owned(),
    }
}

fn redacted_dynamic_tool(params: &serde_json::Map<String, Value>) -> Option<Value> {
    let mut value = serde_json::Map::new();
    for key in ["callId", "tool", "namespace", "arguments"] {
        if let Some(field) = params.get(key) {
            value.insert(key.into(), field.clone());
        }
    }
    value.insert("method".into(), Value::String("item/tool/call".into()));
    bounded(Value::Object(value))
}

fn redacted_questions(params: &serde_json::Map<String, Value>) -> Option<Value> {
    let questions = params.get("questions")?.as_array()?;
    let questions = questions
        .iter()
        .filter_map(|question| {
            let object = question.as_object()?;
            let mut value = serde_json::Map::new();
            for key in ["id", "header", "question"] {
                if let Some(field) = object.get(key).and_then(Value::as_str) {
                    value.insert(key.into(), Value::String(field.into()));
                }
            }
            if let Some(options) = object.get("options").and_then(Value::as_array) {
                let options = options
                    .iter()
                    .filter_map(Value::as_object)
                    .map(|option| {
                        let mut value = serde_json::Map::new();
                        for key in ["label", "description"] {
                            if let Some(field) = option.get(key).and_then(Value::as_str) {
                                value.insert(key.into(), Value::String(field.into()));
                            }
                        }
                        Value::Object(value)
                    })
                    .collect();
                value.insert("options".into(), Value::Array(options));
            }
            if let Some(is_other) = object.get("isOther").and_then(Value::as_bool) {
                value.insert("isOther".into(), Value::Bool(is_other));
            }
            if let Some(multi_select) = object
                .get("multiSelect")
                .or_else(|| object.get("multi_select"))
                .and_then(Value::as_bool)
            {
                value.insert("multiSelect".into(), Value::Bool(multi_select));
            }
            Some(Value::Object(value))
        })
        .collect::<Vec<_>>();
    bounded(Value::Object(serde_json::Map::from_iter([
        ("kind".into(), Value::String("questions".into())),
        ("questions".into(), Value::Array(questions)),
    ])))
}

fn redacted_approval(params: &serde_json::Map<String, Value>, method: &str) -> Option<Value> {
    let mut value = serde_json::Map::new();
    for key in [
        "itemId",
        "callId",
        "reason",
        "cwd",
        "command",
        "grantRoot",
        "permissions",
        "changes",
        "proposedExecpolicyAmendment",
        "proposedNetworkPolicyAmendments",
        "availableDecisions",
    ] {
        if let Some(field) = params.get(key) {
            value.insert(key.into(), field.clone());
        }
    }
    value.insert("method".into(), Value::String(method.into()));
    bounded(Value::Object(value))
}

fn redacted_elicitation(params: &serde_json::Map<String, Value>) -> Option<Value> {
    let mut value = serde_json::Map::new();
    if let Some(message) = params.get("message").and_then(Value::as_str) {
        value.insert("message".into(), Value::String(message.into()));
    }
    if params.get("mode").and_then(Value::as_str) == Some("url") {
        value.insert("kind".into(), Value::String("url".into()));
        for key in ["url", "serverName", "elicitationId"] {
            if let Some(field) = params.get(key) {
                value.insert(key.into(), field.clone());
            }
        }
    } else {
        value.insert("kind".into(), Value::String("questions".into()));
        value.insert(
            "questions".into(),
            Value::Array(elicitation_questions(
                params
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("MCP server requests input"),
                params.get("requestedSchema"),
            )),
        );
    }
    bounded(Value::Object(value))
}

fn elicitation_questions(message: &str, raw_schema: Option<&Value>) -> Vec<Value> {
    let properties = raw_schema
        .and_then(|schema| schema.get("properties"))
        .and_then(Value::as_object);
    let Some(properties) = properties.filter(|properties| !properties.is_empty()) else {
        return vec![json!({
            "id": "mcp",
            "question": message,
            "header": "MCP",
            "multiSelect": false,
            "valueType": "string",
            "options": []
        })];
    };
    properties
        .iter()
        .map(|(id, raw_property)| {
            let property = raw_property.as_object();
            let title = property
                .and_then(|property| property.get("title"))
                .and_then(Value::as_str)
                .filter(|title| !title.is_empty())
                .unwrap_or(id);
            let header = property
                .and_then(|property| property.get("description"))
                .and_then(Value::as_str)
                .filter(|description| !description.is_empty())
                .unwrap_or(message);
            let value_type = property
                .and_then(|property| property.get("type"))
                .and_then(Value::as_str)
                .unwrap_or("string");
            let multi_select = value_type == "array";
            let normalized_type = if multi_select {
                property
                    .and_then(|property| property.get("items"))
                    .and_then(Value::as_object)
                    .and_then(|items| items.get("type"))
                    .and_then(Value::as_str)
                    .unwrap_or("string")
            } else {
                value_type
            };
            json!({
                "id": id,
                "question": title,
                "header": header,
                "multiSelect": multi_select,
                "valueType": normalized_type,
                "options": property.map(elicitation_options).unwrap_or_default()
            })
        })
        .collect()
}

fn elicitation_options(property: &serde_json::Map<String, Value>) -> Vec<Value> {
    let mut options = Vec::new();
    if let Some(values) = property.get("enum").and_then(Value::as_array) {
        options.extend(values.iter().map(
            |value| json!({"label": display_value(value), "description": "", "value": value}),
        ));
    }
    if property.get("type").and_then(Value::as_str) == Some("boolean") {
        options.extend([
            json!({"label":"True","description":"","value":true}),
            json!({"label":"False","description":"","value":false}),
        ]);
    }
    if let Some(values) = property.get("oneOf").and_then(Value::as_array) {
        options.extend(values.iter().filter_map(|value| {
            let object = value.as_object()?;
            let actual = object.get("const").or_else(|| object.get("value"))?;
            let label = object
                .get("title")
                .and_then(Value::as_str)
                .or_else(|| actual.as_str())
                .unwrap_or("");
            Some(json!({
                "label": label,
                "description": object.get("description").and_then(Value::as_str).unwrap_or(""),
                "value": actual
            }))
        }));
    }
    if let Some(items) = property.get("items").and_then(Value::as_object) {
        if let Some(values) = items.get("enum").and_then(Value::as_array) {
            options.extend(values.iter().map(
                |value| json!({"label": display_value(value), "description": "", "value": value}),
            ));
        }
    }
    options
}

fn display_value(value: &Value) -> String {
    value
        .as_str()
        .map_or_else(|| value.to_string(), str::to_owned)
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

fn question_answers(request: &CodexControlRequest, answers: Option<Value>) -> Value {
    let Some(Value::Object(raw_answers)) = answers else {
        return json!({});
    };
    let questions = request
        .input
        .as_ref()
        .and_then(|input| input.get("questions"))
        .and_then(Value::as_array);
    let mut ids_by_visible = std::collections::BTreeMap::new();
    if let Some(questions) = questions {
        for question in questions {
            let Some(question) = question.as_object() else {
                continue;
            };
            let Some(id) = question.get("id").and_then(Value::as_str) else {
                continue;
            };
            if let Some(text) = question.get("question").and_then(Value::as_str) {
                ids_by_visible.insert(text, id);
            }
            ids_by_visible.insert(id, id);
        }
    }
    let mut encoded = serde_json::Map::new();
    for (visible, answer) in raw_answers {
        let id = ids_by_visible
            .get(visible.as_str())
            .copied()
            .unwrap_or(visible.as_str());
        let values = if let Some(answer_values) = answer
            .as_object()
            .and_then(|value| value.get("answers"))
            .and_then(Value::as_array)
        {
            answer_values
                .iter()
                .map(stringify_answer)
                .filter(|value| !value.is_empty())
                .map(Value::String)
                .collect()
        } else {
            match answer {
                Value::Array(values) => values
                    .iter()
                    .map(stringify_answer)
                    .filter(|value| !value.is_empty())
                    .map(Value::String)
                    .collect(),
                Value::Null => Vec::new(),
                value => vec![Value::String(stringify_answer(&value))],
            }
        };
        encoded.insert(id.to_owned(), json!({"answers": values}));
    }
    Value::Object(encoded)
}

fn elicitation_content(request: &CodexControlRequest, answers: Option<Value>) -> Value {
    let Some(Value::Object(mut answers)) = answers else {
        return json!({});
    };
    if let Some(Value::Object(content)) = answers.remove("answers") {
        answers = content;
    }
    let questions = request
        .input
        .as_ref()
        .and_then(|input| input.get("questions"))
        .and_then(Value::as_array);
    let mut ids_by_visible = std::collections::BTreeMap::new();
    let mut details_by_id = std::collections::BTreeMap::new();
    if let Some(questions) = questions {
        for question in questions {
            let Some(question) = question.as_object() else {
                continue;
            };
            let Some(id) = question.get("id").and_then(Value::as_str) else {
                continue;
            };
            if let Some(title) = question.get("question").and_then(Value::as_str) {
                ids_by_visible.insert(title, id);
            }
            ids_by_visible.insert(id, id);
            details_by_id.insert(id, question);
        }
    }
    let mut content = serde_json::Map::new();
    for (visible, answer) in answers {
        let id = ids_by_visible
            .get(visible.as_str())
            .copied()
            .unwrap_or(visible.as_str());
        let question = details_by_id.get(id).copied();
        let answer = unwrap_answer(answer);
        let answer = map_option_value(question, answer);
        content.insert(id.to_owned(), coerce_answer(question, answer));
    }
    Value::Object(content)
}

fn unwrap_answer(value: Value) -> Value {
    value
        .as_object()
        .and_then(|value| value.get("answers"))
        .cloned()
        .unwrap_or(value)
}

fn map_option_value(question: Option<&serde_json::Map<String, Value>>, value: Value) -> Value {
    let Some(options) = question
        .and_then(|question| question.get("options"))
        .and_then(Value::as_array)
    else {
        return value;
    };
    let map_one = |value: Value| {
        let label = value.as_str().unwrap_or_default();
        options
            .iter()
            .filter_map(Value::as_object)
            .find(|option| option.get("label").and_then(Value::as_str) == Some(label))
            .and_then(|option| option.get("value"))
            .cloned()
            .unwrap_or(value)
    };
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(map_one).collect()),
        value => map_one(value),
    }
}

fn coerce_answer(question: Option<&serde_json::Map<String, Value>>, value: Value) -> Value {
    let value_type = question
        .and_then(|question| question.get("valueType"))
        .and_then(Value::as_str)
        .unwrap_or("string");
    let coerce_one = |value: Value| match (value_type, value) {
        ("boolean", Value::String(value)) if value == "true" => Value::Bool(true),
        ("boolean", Value::String(value)) if value == "false" => Value::Bool(false),
        ("number", Value::String(value)) => value
            .parse::<f64>()
            .ok()
            .and_then(|value| serde_json::Number::from_f64(value).map(Value::Number))
            .unwrap_or(Value::String(value)),
        (_, value) => value,
    };
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(coerce_one).collect()),
        value => coerce_one(value),
    }
}

fn stringify_answer(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Null => String::new(),
        value => value.to_string(),
    }
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
