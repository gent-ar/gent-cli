use serde_json::{Value, json};

use super::bounded;

pub(super) fn dynamic_tool_name(params: &serde_json::Map<String, Value>) -> String {
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

pub(super) fn redacted_dynamic_tool(params: &serde_json::Map<String, Value>) -> Option<Value> {
    let mut value = serde_json::Map::new();
    for key in ["callId", "tool", "namespace", "arguments"] {
        if let Some(field) = params.get(key) {
            value.insert(key.into(), field.clone());
        }
    }
    value.insert("method".into(), Value::String("item/tool/call".into()));
    bounded(Value::Object(value))
}

pub(super) fn redacted_questions(params: &serde_json::Map<String, Value>) -> Option<Value> {
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

pub(super) fn redacted_approval(
    params: &serde_json::Map<String, Value>,
    method: &str,
) -> Option<Value> {
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

pub(super) fn redacted_elicitation(params: &serde_json::Map<String, Value>) -> Option<Value> {
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
