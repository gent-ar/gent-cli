use serde_json::{Value, json};

use super::CodexControlRequest;

pub(super) fn question_answers(request: &CodexControlRequest, answers: Option<Value>) -> Value {
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

pub(super) fn elicitation_content(request: &CodexControlRequest, answers: Option<Value>) -> Value {
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
