//! Claude frames that need strict frame-local tool identity checks.

use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, ProviderFailureClassification,
    ToolActivity, ToolPhase,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

use super::PublicWireFact;

#[path = "claude_protocol/usage.rs"]
mod usage;
pub(super) use usage::context_usage;
#[path = "claude_protocol/control.rs"]
mod control;
pub(super) use control::control_response;

pub(super) fn partial_text(delta: &Value, field: &str, thinking: bool) -> Vec<PublicWireFact> {
    let Some(text) = string(delta, field).filter(|text| !text.is_empty()) else {
        return diagnostic("malformedClaudeContentBlockDelta");
    };
    let event = if thinking {
        NormalizedProviderEvent::Thinking {
            text: text.into(),
            is_partial: true,
        }
    } else {
        NormalizedProviderEvent::Output {
            text: text.into(),
            is_partial: true,
        }
    };
    vec![PublicWireFact::Event(event)]
}

pub(super) fn failure_classification(frame: &Value) -> ProviderFailureClassification {
    let text = [
        frame.get("message").and_then(Value::as_str),
        frame.pointer("/error/message").and_then(Value::as_str),
        frame.pointer("/error/type").and_then(Value::as_str),
        frame.get("result").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();
    if text.contains("401") || text.contains("unauthor") || text.contains("authentication") {
        ProviderFailureClassification::Authentication
    } else if text.contains("rate") || text.contains("limit") {
        ProviderFailureClassification::RateLimited
    } else if text.contains("context") || text.contains("token limit") {
        ProviderFailureClassification::ContextLimit
    } else {
        ProviderFailureClassification::Provider
    }
}

pub(super) fn failure_message(frame: &Value) -> String {
    let text = [
        frame.get("message").and_then(Value::as_str),
        frame.pointer("/error/message").and_then(Value::as_str),
        frame.get("result").and_then(Value::as_str),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(" ")
    .to_ascii_lowercase();
    if text.contains("401") || text.contains("unauthor") || text.contains("authentication") {
        "Claude authentication failed.".into()
    } else if text.contains("rate") || text.contains("limit") {
        "Claude rate limit reached.".into()
    } else if text.contains("context") || text.contains("token limit") {
        "Claude context limit reached.".into()
    } else {
        "Claude ended the turn with an error.".into()
    }
}

/// Reduces echoed tool results only if this one frame proves both identifier and display name.
/// Real Claude result blocks normally omit the name, so runner-owned correlation must handle
/// them; this pure reducer must not invent one.
pub(super) fn user(frame: &Value) -> Vec<PublicWireFact> {
    let Some(content) = frame.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    let mut facts = content
        .iter()
        .filter(|block| string(block, "type") == Some("tool_result"))
        .flat_map(tool_result)
        .collect::<Vec<_>>();
    facts.extend(background_launches(frame));
    facts
}

fn tool_result(block: &Value) -> Vec<PublicWireFact> {
    let Some(tool_use_id) = string(block, "tool_use_id").filter(|id| !id.is_empty()) else {
        return diagnostic("malformedClaudeToolResult");
    };
    let Some(tool_name) = string(block, "tool_name")
        .or_else(|| string(block, "name"))
        .filter(|name| !name.is_empty())
    else {
        return diagnostic("unresolvedClaudeToolResult");
    };
    let phase = if block.get("is_error").and_then(Value::as_bool) == Some(true) {
        ToolPhase::Failed
    } else {
        ToolPhase::Completed
    };
    activity(
        tool_use_id,
        tool_name,
        phase,
        block.get("content").map(digest_json),
    )
}

pub(crate) fn background_launches(frame: &Value) -> Vec<PublicWireFact> {
    let Some(content) = frame.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|block| string(block, "type") == Some("tool_result"))
        .filter_map(|block| {
            let parent_tool_use_id = string(block, "tool_use_id").filter(|id| !id.is_empty())?;
            let content = block.get("content")?;
            let text = content_text(content);
            let lower = text.to_ascii_lowercase();
            let launch_phrase = lower.contains("async agent launched successfully")
                || lower.contains("running in background")
                || lower.contains("working in the background")
                || lower.contains("background job");
            if !launch_phrase || !lower.contains("output_file:") {
                return None;
            }
            let child_id = marker_value(&text, "agentId:")?;
            Some(PublicWireFact::Event(
                NormalizedProviderEvent::ChildStarted {
                    child_id: child_id.into(),
                    parent_tool_use_id: parent_tool_use_id.into(),
                },
            ))
        })
        .collect()
}

fn content_text(value: &Value) -> String {
    match value {
        Value::String(text) => text.clone(),
        Value::Array(blocks) => blocks
            .iter()
            .filter_map(|block| {
                (string(block, "type") == Some("text"))
                    .then(|| string(block, "text"))
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn marker_value<'a>(text: &'a str, marker: &str) -> Option<&'a str> {
    let start = text.find(marker)? + marker.len();
    let value = text[start..].split_whitespace().next()?;
    (!value.is_empty()).then_some(value)
}

/// Background task notifications normally include only a parent tool id. Preserve no content;
/// normalize only compatible frames that also prove the display name.
pub(super) fn background_activity(frame: &Value) -> Vec<PublicWireFact> {
    let Some(tool_use_id) = string(frame, "tool_use_id").filter(|id| !id.is_empty()) else {
        return diagnostic("malformedClaudeBackgroundTask");
    };
    let Some(tool_name) = string(frame, "tool_name")
        .or_else(|| string(frame, "name"))
        .filter(|name| !name.is_empty())
    else {
        return diagnostic("unresolvedClaudeBackgroundTask");
    };
    activity(tool_use_id, tool_name, ToolPhase::Started, None)
}

pub(crate) fn background_terminal_tool_use_ids(frame: &Value) -> Vec<String> {
    let content = match string(frame, "type") {
        Some("queue-operation") if string(frame, "operation") == Some("enqueue") => {
            string(frame, "content")
        }
        Some("user") => frame.pointer("/message/content").and_then(Value::as_str),
        _ => None,
    };
    let Some(content) = content else {
        return Vec::new();
    };
    let mut ids = Vec::new();
    let mut remaining = content;
    while let Some(start) = remaining.find("<task-notification>") {
        remaining = &remaining[start + "<task-notification>".len()..];
        let Some(end) = remaining.find("</task-notification>") else {
            break;
        };
        let body = &remaining[..end];
        if tag(body, "status").is_some_and(|status| status.trim() == "completed") {
            if let Some(id) = tag(body, "tool-use-id")
                .map(str::trim)
                .filter(|id| !id.is_empty() && !id.chars().any(char::is_whitespace))
            {
                if !ids.iter().any(|known| known == id) {
                    ids.push(id.to_owned());
                }
            }
        }
        remaining = &remaining[end + "</task-notification>".len()..];
    }
    ids
}

pub(crate) fn correlated_background_activity(
    tool_names: &BTreeMap<String, String>,
    frame: &Value,
) -> Option<Vec<PublicWireFact>> {
    if string(frame, "type") != Some("system")
        || !matches!(
            string(frame, "subtype"),
            Some("task_started" | "task_progress")
        )
        || string(frame, "tool_name").is_some_and(|name| !name.is_empty())
    {
        return None;
    }
    let tool_use_id = string(frame, "tool_use_id").filter(|id| !id.is_empty())?;
    let tool_name = tool_names.get(tool_use_id)?;
    Some(activity(tool_use_id, tool_name, ToolPhase::Started, None))
}

pub(crate) fn activity(
    tool_use_id: &str,
    tool_name: &str,
    phase: ToolPhase,
    output_digest: Option<String>,
) -> Vec<PublicWireFact> {
    vec![PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ToolActivity {
            activity: ToolActivity {
                tool_use_id: tool_use_id.into(),
                tool_name: tool_name.into(),
                phase,
                output_digest,
            },
        },
    )]
}

fn digest_json(value: &Value) -> String {
    format!("sha256:{:x}", Sha256::digest(value.to_string().as_bytes()))
}

fn diagnostic(classification: &str) -> Vec<PublicWireFact> {
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: classification.into(),
        },
    )]
}

fn string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}

fn tag<'a>(value: &'a str, name: &str) -> Option<&'a str> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let start = value.find(&open)? + open.len();
    let end = value[start..].find(&close)? + start;
    Some(&value[start..end])
}

#[cfg(test)]
mod tests {
    use super::background_terminal_tool_use_ids;
    use serde_json::json;

    #[test]
    fn accepts_only_explicit_completed_background_notifications() {
        let notification = "<task-notification>\n<tool-use-id>task-1</tool-use-id>\n<status> completed </status>\n</task-notification>";
        assert_eq!(
            background_terminal_tool_use_ids(&json!({
                "type": "queue-operation",
                "operation": "enqueue",
                "content": notification
            })),
            ["task-1"]
        );
        assert_eq!(
            background_terminal_tool_use_ids(&json!({
                "type": "user",
                "message": {"content": notification}
            })),
            ["task-1"]
        );
        assert!(background_terminal_tool_use_ids(&json!({
            "type": "user",
            "message": {"content": "<task-notification><tool-use-id>task-1</tool-use-id><status>running</status></task-notification>"}
        }))
        .is_empty());
    }

    #[test]
    fn rejects_unbounded_or_malformed_task_ids() {
        let notification = "<task-notification><tool-use-id>task one</tool-use-id><status>completed</status></task-notification>";
        assert!(
            background_terminal_tool_use_ids(&json!({
                "type": "queue-operation",
                "operation": "enqueue",
                "content": notification
            }))
            .is_empty()
        );
    }
}
