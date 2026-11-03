//! The shared presentation boundary for operation arguments from any adapter.

use serde_json::{Map, Value};

const MAX_BYTES: usize = 16 * 1024;
const MAX_STRING_BYTES: usize = 4096;
const TRUNCATED: &str = "[truncated]";
const REDACTED: &str = "[redacted]";

/// Retains useful structured arguments while bounding nesting, allocation and wire size.
/// Known credential fields and common credential tokens are redacted. This is a display
/// preview, not a credential scanner or a replacement for the runner's original input.
#[must_use]
pub fn operation_preview(input: &Value) -> Option<Value> {
    operation_preview_with_limit(input, MAX_BYTES)
}

pub fn operation_preview_with_limit(input: &Value, byte_limit: usize) -> Option<Value> {
    if byte_limit < 64 {
        return None;
    }
    if !input.is_object() || input.as_object().is_some_and(Map::is_empty) {
        return None;
    }
    // Transport metadata, environment maps and provider internals are not operation details.
    // Adapters normalize custom tools into these common presentation fields.
    if !input.as_object()?.keys().any(|key| is_operation_field(key)) {
        return None;
    }
    let mut remaining = byte_limit.min(MAX_BYTES);
    Some(preview(input, 0, &mut remaining))
}

fn is_operation_field(key: &str) -> bool {
    matches!(
        key,
        "command"
            | "commands"
            | "cwd"
            | "workdir"
            | "file_path"
            | "path"
            | "paths"
            | "content"
            | "old_string"
            | "new_string"
            | "oldText"
            | "newText"
            | "replace_all"
            | "changes"
            | "diff"
            | "patch"
            | "pattern"
            | "query"
            | "url"
            | "description"
            | "questions"
            | "question"
            | "prompt"
            | "options"
            | "plan"
            | "arguments"
            | "tool"
            | "namespace"
            | "reason"
            | "permissions"
            | "grantRoot"
            | "method"
            | "kind"
            | "limit"
            | "offset"
            | "timeout"
            | "run_in_background"
    )
}

fn preview(value: &Value, depth: usize, remaining: &mut usize) -> Value {
    if depth >= 6 || *remaining < 64 {
        *remaining = remaining.saturating_sub(TRUNCATED.len() + 2);
        return Value::String(TRUNCATED.into());
    }
    match value {
        Value::Object(fields) => {
            *remaining -= 2;
            let mut result = Map::new();
            let mut fields = fields
                .iter()
                .filter(|(key, _)| depth != 0 || is_operation_field(key))
                .take(33)
                .collect::<Vec<_>>();
            fields.sort_by_key(|(key, _)| match key.as_str() {
                "file_path" | "path" | "paths" | "cwd" | "workdir" | "move_path" => 0,
                "command" | "commands" | "kind" => 1,
                "content" | "old_string" | "new_string" | "diff" | "patch" => 2,
                _ => 3,
            });
            for (index, (key, value)) in fields.into_iter().enumerate() {
                if index >= 32 || key.len() > 256 || *remaining < key.len() * 6 + 128 {
                    result.insert("_previewTruncated".into(), Value::Bool(true));
                    *remaining = remaining.saturating_sub(32);
                    break;
                }
                *remaining -= key.len() * 6 + 4;
                let value = if sensitive_key(key) {
                    *remaining -= REDACTED.len() + 2;
                    Value::String(REDACTED.into())
                } else {
                    preview(value, depth + 1, remaining)
                };
                result.insert(key.clone(), value);
            }
            Value::Object(result)
        }
        Value::Array(values) => {
            *remaining -= 2;
            let mut result = Vec::new();
            for (index, value) in values.iter().enumerate() {
                if index >= 32 || *remaining < 64 {
                    result.push(Value::String(TRUNCATED.into()));
                    *remaining = remaining.saturating_sub(TRUNCATED.len() + 3);
                    break;
                }
                *remaining -= 1;
                result.push(preview(value, depth + 1, remaining));
            }
            Value::Array(result)
        }
        Value::String(text) => {
            // JSON escaping can expand one byte to six. Reserve the marker too.
            let limit = MAX_STRING_BYTES.min((*remaining - 32) / 6);
            let mut end = text.len().min(limit);
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            let mut text = redact_text(&text[..end]);
            if end < value.as_str().unwrap_or_default().len() {
                text.push_str(TRUNCATED);
            }
            *remaining = remaining.saturating_sub(text.len() * 6 + 2);
            Value::String(text)
        }
        _ => {
            *remaining -= 32;
            value.clone()
        }
    }
}

fn sensitive_key(key: &str) -> bool {
    let key = key.to_ascii_lowercase().replace(['_', '-'], "");
    [
        "password",
        "passwd",
        "secret",
        "authorization",
        "apikey",
        "accesskey",
        "accesstoken",
        "refreshtoken",
        "credential",
        "cookie",
        "privatekey",
    ]
    .iter()
    .any(|needle| key.contains(needle))
        || key.ends_with("token")
}

fn redact_text(text: &str) -> String {
    let mut result = String::new();
    let mut redact_next = false;
    // Preserve newlines and indentation in file previews and shell scripts.
    for part in text.split_inclusive(char::is_whitespace) {
        let token = part.trim_end_matches(char::is_whitespace);
        let whitespace = &part[token.len()..];
        if token.is_empty() {
            result.push_str(whitespace);
            continue;
        }
        let lower = token.trim_matches(['\'', '"']).to_ascii_lowercase();
        let assignment = lower.split_once('=').or_else(|| lower.split_once(':'));
        let secret_assignment =
            assignment.is_some_and(|(key, value)| sensitive_key(key) && !value.is_empty());
        let secret_token = ["sk-", "ghp_", "github_pat_", "xoxb-", "xoxp-"]
            .iter()
            .any(|prefix| lower.starts_with(prefix));
        if redact_next || secret_assignment || secret_token {
            result.push_str(REDACTED);
        } else {
            result.push_str(token);
        }
        redact_next = lower == "bearer"
            || sensitive_key(lower.trim_end_matches([':', '=']))
                && (lower.starts_with("--") || lower.ends_with([':', '=']));
        result.push_str(whitespace);
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ToolActivity;
    use serde_json::json;

    #[test]
    fn structured_preview_preserves_edits_and_redacts_credentials() {
        let preview = operation_preview(&json!({"file_path":"X:/repo/main.rs", "content":"fn main() {\n    println!(\"hello\");\n}\n", "arguments":{"headers":{"Authorization":"Bearer private"}}, "command":"curl --token private\n  echo ok", "token":"private"})).unwrap();
        assert_eq!(preview["file_path"], "X:/repo/main.rs");
        assert_eq!(
            preview["content"],
            "fn main() {\n    println!(\"hello\");\n}\n"
        );
        assert_eq!(preview["arguments"]["headers"]["Authorization"], REDACTED);
        assert_eq!(preview["command"], "curl --token [redacted]\n  echo ok");
        assert!(!preview.to_string().contains("private"));
    }

    #[test]
    fn singular_question_keeps_visible_text_and_options() {
        let preview = operation_preview(
            &json!({"question":"Which option?", "options":[{"label":"First"}], "token":"private"}),
        )
        .unwrap();
        assert_eq!(preview["question"], "Which option?");
        assert_eq!(preview["options"][0]["label"], "First");
        assert!(!preview.to_string().contains("private"));
    }

    #[test]
    fn previews_are_bounded_utf8_safe_and_mark_truncation() {
        let input = json!({"content": "界\n".repeat(100_000), "arguments": vec![json!({"text": "a".repeat(100_000)}); 100]});
        let preview = operation_preview(&input).unwrap();
        let encoded = serde_json::to_vec(&preview).unwrap();
        assert!(encoded.len() <= MAX_BYTES, "{}", encoded.len());
        assert!(preview["content"].as_str().unwrap().ends_with(TRUNCATED));
    }

    #[test]
    fn old_tool_facts_deserialize_without_input_and_new_facts_round_trip() {
        let old =
            json!({"toolUseId":"t", "toolName":"Write", "phase":"started", "outputDigest":null});
        let mut activity: ToolActivity = serde_json::from_value(old.clone()).unwrap();
        assert_eq!(activity.input, None);
        assert_eq!(serde_json::to_value(&activity).unwrap(), old);
        activity.input = operation_preview(&json!({"file_path":"main.rs", "content":"new"}));
        assert_eq!(
            serde_json::from_value::<ToolActivity>(serde_json::to_value(&activity).unwrap())
                .unwrap(),
            activity
        );
    }
}
