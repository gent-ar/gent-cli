//! Claude frames that need strict frame-local tool identity checks.

use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, ProviderFailureClassification,
    RootActivity, ToolActivity, ToolPhase, TurnPhase,
};
use serde_json::Value;
use sha2::{Digest, Sha256};

use super::PublicWireFact;

#[path = "claude_protocol/usage.rs"]
mod usage;
pub(super) use usage::{context_usage, turn_usage};

pub(super) fn compaction(frame: &Value) -> Vec<PublicWireFact> {
    let observation = match (string(frame, "subtype"), frame.get("compact_result")) {
        (Some("compact_boundary"), _) => super::PublicCompactionObservation::Completed,
        (_, Some(result)) if result.as_str() != Some("success") => {
            super::PublicCompactionObservation::Failed {
                failure: gent_types::AgentChatCompactionFailure::ProviderFailed,
            }
        }
        _ if string(frame, "status") == Some("compacting") => {
            super::PublicCompactionObservation::Started
        }
        _ => return Vec::new(),
    };
    vec![PublicWireFact::Compaction(observation)]
}

pub(super) fn proposed_plan(block: &Value) -> Option<PublicWireFact> {
    (string(block, "name") == Some("ExitPlanMode"))
        .then(|| block.pointer("/input/plan").and_then(Value::as_str))
        .flatten()
        .filter(|plan| !plan.trim().is_empty())
        .map(|plan| {
            PublicWireFact::Event(gent_types::NormalizedProviderEvent::PlanProposed {
                text: plan.into(),
            })
        })
}
#[path = "claude_protocol/control.rs"]
mod control;
pub(super) use control::control_response;
#[path = "claude_protocol/child.rs"]
mod child;
pub(crate) use child::{child, child_parent, task_notification, task_terminal};

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
    let parent_tool_use_id = child_parent(frame);
    let mut facts = content
        .iter()
        .filter(|block| string(block, "type") == Some("tool_result"))
        .flat_map(|block| tool_result(block, parent_tool_use_id))
        .collect::<Vec<_>>();
    facts.extend(background_launches(frame));
    facts
}

fn tool_result(block: &Value, parent_tool_use_id: Option<&str>) -> Vec<PublicWireFact> {
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
    let mut facts = activity(
        tool_use_id,
        tool_name,
        phase,
        block.get("content").map(digest_json),
        parent_tool_use_id,
    );
    let text = block.get("content").map(content_text).unwrap_or_default();
    if parent_tool_use_id.is_none() && !text.is_empty() {
        facts.push(PublicWireFact::Event(
            NormalizedProviderEvent::ToolOutputDelta {
                tool_use_id: tool_use_id.into(),
                text,
                is_partial: false,
            },
        ));
    }
    facts
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

pub(crate) fn content_text(value: &Value) -> String {
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

pub(crate) fn activity(
    tool_use_id: &str,
    tool_name: &str,
    phase: ToolPhase,
    output_digest: Option<String>,
    parent_tool_use_id: Option<&str>,
) -> Vec<PublicWireFact> {
    vec![PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::ToolActivity {
            activity: ToolActivity {
                tool_use_id: tool_use_id.into(),
                tool_name: tool_name.into(),
                phase,
                output_digest,
                parent_tool_use_id: parent_tool_use_id.map(str::to_owned),
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

pub(super) fn result(frame: &Value) -> Vec<PublicWireFact> {
    let failed = frame
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let phase = if failed {
        TurnPhase::Failed
    } else {
        TurnPhase::Ready
    };
    let mut facts: Vec<PublicWireFact> = turn_usage(frame)
        .map(PublicWireFact::Event)
        .into_iter()
        .collect();
    if failed {
        facts.push(PublicWireFact::Event(
            NormalizedProviderEvent::ProviderFailure {
                classification: failure_classification(frame),
                message: failure_message(frame),
            },
        ));
    }
    facts.extend([
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootActivity {
            activity: RootActivity::Idle,
        }),
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase { phase }),
    ]);
    facts
}
