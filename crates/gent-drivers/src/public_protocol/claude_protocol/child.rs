use gent_types::{NormalizedProviderEvent, ToolPhase, WorkPhase};
use serde_json::Value;

use super::super::PublicWireFact;
use super::{activity, diagnostic, string, user};

pub(crate) fn child_parent(frame: &Value) -> Option<&str> {
    string(frame, "parent_tool_use_id").filter(|id| !id.is_empty())
}

pub(crate) fn child(frame: &Value, parent_tool_use_id: &str) -> Vec<PublicWireFact> {
    match string(frame, "type") {
        Some("assistant") => frame
            .pointer("/message/content")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .flat_map(|block| child_content(block, parent_tool_use_id))
            .collect(),
        Some("user") => user(frame),
        _ => Vec::new(),
    }
}

fn child_content(block: &Value, parent_tool_use_id: &str) -> Vec<PublicWireFact> {
    match string(block, "type") {
        Some("tool_use") => match (string(block, "id"), string(block, "name")) {
            (Some(tool_use_id), Some(tool_name))
                if !tool_use_id.is_empty() && !tool_name.is_empty() =>
            {
                activity(
                    tool_use_id,
                    tool_name,
                    ToolPhase::Started,
                    None,
                    Some(parent_tool_use_id),
                )
            }
            _ => diagnostic("malformedClaudeToolUse"),
        },
        Some("text") => string(block, "text")
            .filter(|text| !text.is_empty())
            .map(|text| {
                PublicWireFact::Event(NormalizedProviderEvent::ToolOutputDelta {
                    tool_use_id: parent_tool_use_id.into(),
                    text: text.into(),
                    is_partial: false,
                })
            })
            .into_iter()
            .collect(),
        _ => Vec::new(),
    }
}

pub(crate) fn task_terminal(frame: &Value) -> Option<WorkPhase> {
    match string(frame, "status")? {
        "completed" => Some(WorkPhase::Done),
        "failed" => Some(WorkPhase::Failed),
        "stopped" => Some(WorkPhase::Interrupted),
        _ => None,
    }
}

pub(crate) fn task_notification(frame: &Value) -> Vec<PublicWireFact> {
    if task_terminal(frame).is_some() {
        Vec::new()
    } else {
        diagnostic("unsupportedClaudeTaskStatus")
    }
}
