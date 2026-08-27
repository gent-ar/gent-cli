use gent_types::{NormalizedProviderEvent, ProviderFailureClassification};
use serde_json::Value;

use super::PublicWireFact;
use super::support::diagnostic;

pub(super) fn error(frame: &Value) -> Vec<PublicWireFact> {
    let text = [
        frame
            .pointer("/params/error/message")
            .and_then(Value::as_str),
        frame.pointer("/params/msg/message").and_then(Value::as_str),
        frame.pointer("/params/message").and_then(Value::as_str),
        frame
            .pointer("/params/error/codexErrorInfo")
            .and_then(Value::as_str),
        frame
            .pointer("/params/error/codex_error_info")
            .and_then(Value::as_str),
        frame
            .pointer("/params/msg/codex_error_info")
            .and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();
    let (classification, message) =
        if text.contains("401") || text.contains("unauthor") || text.contains("missing bearer") {
            (
                ProviderFailureClassification::Authentication,
                "Codex authentication failed.",
            )
        } else if text.contains("rate") || text.contains("limit") {
            (
                ProviderFailureClassification::RateLimited,
                "Codex rate limit reached.",
            )
        } else if text.contains("context") || text.contains("token limit") {
            (
                ProviderFailureClassification::ContextLimit,
                "Codex context limit reached.",
            )
        } else {
            (
                ProviderFailureClassification::Provider,
                "Codex reported a provider error.",
            )
        };
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::ProviderFailure {
            classification,
            message: message.into(),
        },
    )]
}

pub(super) fn mcp_progress(frame: &Value) -> Vec<PublicWireFact> {
    let item_id = frame.pointer("/params/itemId").and_then(Value::as_str);
    let message = frame.pointer("/params/message").and_then(Value::as_str);
    match (item_id, message) {
        (Some(tool_use_id), Some(message)) if !tool_use_id.is_empty() && !message.is_empty() => {
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::ToolOutputDelta {
                    tool_use_id: tool_use_id.into(),
                    text: message.into(),
                    is_partial: true,
                },
            )]
        }
        _ => diagnostic("malformedCodexMcpProgress"),
    }
}

pub(super) fn tool_output(frame: &Value) -> Vec<PublicWireFact> {
    let tool_use_id = frame.pointer("/params/itemId").and_then(Value::as_str);
    let text = frame
        .pointer("/params/delta")
        .or_else(|| frame.pointer("/params/stdin"))
        .and_then(Value::as_str)
        .map(str::to_owned)
        .or_else(|| diff_from_changes(frame.pointer("/params/changes")));
    match (tool_use_id, text) {
        (Some(tool_use_id), Some(text)) if !tool_use_id.is_empty() && !text.is_empty() => {
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::ToolOutputDelta {
                    tool_use_id: tool_use_id.into(),
                    text,
                    is_partial: true,
                },
            )]
        }
        _ => diagnostic("malformedCodexToolOutputDelta"),
    }
}

fn diff_from_changes(changes: Option<&Value>) -> Option<String> {
    let values = changes?.as_array()?;
    let mut parts = Vec::new();
    for change in values {
        let Some(change) = change.as_object() else {
            continue;
        };
        for key in ["diff", "patch", "content"] {
            if let Some(text) = change
                .get(key)
                .and_then(Value::as_str)
                .filter(|text| !text.is_empty())
            {
                parts.push(text);
                break;
            }
        }
    }
    (!parts.is_empty()).then(|| parts.join("\n"))
}
