//! Claude frames that need strict frame-local tool identity checks.

use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolActivity, ToolPhase};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::PublicWireFact;

/// Reduces echoed tool results only if this one frame proves both identifier and display name.
/// Real Claude result blocks normally omit the name, so runner-owned correlation must handle
/// them; this pure reducer must not invent one.
pub(super) fn user(frame: &Value) -> Vec<PublicWireFact> {
    let Some(content) = frame.pointer("/message/content").and_then(Value::as_array) else {
        return Vec::new();
    };
    content
        .iter()
        .filter(|block| string(block, "type") == Some("tool_result"))
        .flat_map(tool_result)
        .collect()
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

fn activity(
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
