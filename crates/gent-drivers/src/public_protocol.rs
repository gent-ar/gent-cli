//! Pure, documented Claude stream-JSON and Codex app-server frame normalization.

use crate::PublicProvider;
use gent_types::{
    AgentChatCompactionFailure, NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity,
    ToolPhase,
};
use serde_json::Value;

pub(crate) mod claude_protocol;

pub(crate) fn oversized_frames_skipped(skipped: usize) -> Option<PublicWireFact> {
    (skipped > 0).then(|| {
        PublicWireFact::Event(gent_types::NormalizedProviderEvent::TransportDiagnostic {
            classification: gent_types::OVERSIZED_PROVIDER_FRAME_DIAGNOSTIC.into(),
        })
    })
}
mod codex_protocol;

/// A provider-neutral fact extracted without process, ledger, or UI access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicWireFact {
    SessionStarted {
        provider_session_id: String,
    },
    Event(NormalizedProviderEvent),
    Lifecycle(NormalizedLifecycleSignal),
    /// A provider-neutral compaction transition with no provider-native identifiers.
    ///
    /// The daemon must bind this to its owned prompt turn and create its own durable event ID
    /// before it may reach the private compaction ingress.
    Compaction(PublicCompactionObservation),
}

/// A normalized provider compaction transition with all provider-native detail discarded.
///
/// This deliberately has no session, thread, item, or provider event identity. Those values
/// never leave the runner; the daemon assigns a durable source identity after ownership checks.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicCompactionObservation {
    Started,
    Completed,
    Failed { failure: AgentChatCompactionFailure },
}

/// Normalizes one documented public-provider frame. Unknown fields never enter the result.
#[must_use]
pub fn normalize_public_frame(provider: PublicProvider, frame: &Value) -> Vec<PublicWireFact> {
    match provider {
        PublicProvider::Claude => claude(frame),
        PublicProvider::Codex => codex_protocol::normalize(frame),
    }
}

/// Replays an ordered synthetic or recorded transcript through the pure provider contract.
#[must_use]
pub fn replay_public_frames(provider: PublicProvider, frames: &[Value]) -> Vec<PublicWireFact> {
    frames
        .iter()
        .flat_map(|frame| normalize_public_frame(provider, frame))
        .collect()
}

fn claude(frame: &Value) -> Vec<PublicWireFact> {
    if let Some(parent_tool_use_id) = claude_protocol::child_parent(frame) {
        return claude_protocol::child(frame, parent_tool_use_id);
    }
    match (string(frame, "type"), string(frame, "subtype")) {
        (Some("system"), Some("init")) => session(frame, "session_id", "malformedClaudeInit"),
        (Some("system"), Some("permission_denied")) => permission_denied(frame),
        (Some("system"), Some("task_notification")) => claude_protocol::task_notification(frame),
        (
            Some("system"),
            Some(
                "task_started"
                | "task_progress"
                | "task_updated"
                | "background_tasks_changed"
                | "thinking_tokens",
            ),
        ) => Vec::new(),
        (Some("system"), Some("status" | "compact_boundary")) => claude_protocol::compaction(frame),
        (Some("stream_event"), _) => claude_stream_event(frame),
        (Some("assistant"), _) => claude_assistant(frame),
        (Some("user"), _) => claude_protocol::user(frame),
        (Some("control_response"), _) => claude_protocol::control_response(frame),
        (Some("control_cancel_request" | "tool_progress" | "rate_limit_event"), _) => Vec::new(),
        (Some("error"), _) => claude_error(frame),
        (Some("result"), _) => claude_protocol::result(frame),
        _ => diagnostic("unsupportedClaudeFrame"),
    }
}

/// Reduces Claude's nested streaming envelope without retaining provider-native payloads.
///
/// The public runner already owns the durable Gent turn, so Claude's `message_start` cannot
/// create a second turn from its opaque provider message id. It is nevertheless liveness proof.
fn claude_stream_event(frame: &Value) -> Vec<PublicWireFact> {
    let Some(event) = frame.get("event") else {
        return diagnostic("malformedClaudeStreamEvent");
    };
    match string(event, "type") {
        Some("message_start") => {
            let mut facts = vec![PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::RootActivity {
                    activity: RootActivity::Generating,
                },
            )];
            if let Some(usage) = claude_protocol::context_usage(event) {
                facts.push(PublicWireFact::Event(usage));
            }
            facts
        }
        Some("content_block_start") => claude_stream_block_start(event),
        Some("content_block_delta") => claude_stream_delta(event),
        Some(
            "content_block_stop" | "message_stop" | "message_delta" | "ping" | "signature_delta",
        ) => Vec::new(),
        Some("error") => claude_error(event),
        Some(_) => diagnostic("unsupportedClaudeStreamEvent"),
        None => diagnostic("malformedClaudeStreamEvent"),
    }
}

fn claude_stream_block_start(event: &Value) -> Vec<PublicWireFact> {
    let Some(block) = event.get("content_block") else {
        return diagnostic("malformedClaudeContentBlockStart");
    };
    match string(block, "type") {
        Some("tool_use") => {
            tool_activity(block, ToolPhase::Started, "malformedClaudeToolUse", false)
        }
        Some("text" | "thinking") => Vec::new(),
        Some(_) => diagnostic("unsupportedClaudeContentBlock"),
        None => diagnostic("malformedClaudeContentBlockStart"),
    }
}

fn claude_stream_delta(event: &Value) -> Vec<PublicWireFact> {
    let Some(delta) = event.get("delta") else {
        return diagnostic("malformedClaudeContentBlockDelta");
    };
    match string(delta, "type") {
        Some("text_delta") => claude_protocol::partial_text(delta, "text", false),
        Some("thinking_delta") => claude_protocol::partial_text(delta, "thinking", true),
        Some("input_json_delta") => {
            let Some(partial_json) =
                string(delta, "partial_json").filter(|value| !value.is_empty())
            else {
                return diagnostic("malformedClaudeContentBlockDelta");
            };
            let Some(block_index) = event.get("index").and_then(Value::as_u64) else {
                return diagnostic("malformedClaudeContentBlockDelta");
            };
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::ToolInputDelta {
                    block_index,
                    partial_json: partial_json.into(),
                },
            )]
        }
        Some(_) => diagnostic("unsupportedClaudeContentBlockDelta"),
        None => diagnostic("malformedClaudeContentBlockDelta"),
    }
}

fn session(frame: &Value, field: &str, invalid: &str) -> Vec<PublicWireFact> {
    string(frame, field)
        .filter(|id| !id.is_empty())
        .map_or_else(
            || diagnostic(invalid),
            |provider_session_id| {
                vec![PublicWireFact::SessionStarted {
                    provider_session_id: provider_session_id.into(),
                }]
            },
        )
}

fn claude_assistant(frame: &Value) -> Vec<PublicWireFact> {
    let Some(content) = frame.pointer("/message/content").and_then(Value::as_array) else {
        return diagnostic("malformedClaudeAssistant");
    };
    let visible: Vec<_> = content
        .iter()
        .filter(|block| !signed_empty_thinking(block))
        .collect();
    if visible.is_empty() && !content.is_empty() {
        return Vec::new();
    }
    let facts: Vec<_> = visible.into_iter().flat_map(claude_content).collect();
    if facts.is_empty() {
        diagnostic("emptyClaudeAssistant")
    } else {
        facts
    }
}

fn signed_empty_thinking(block: &Value) -> bool {
    string(block, "type") == Some("thinking")
        && string(block, "thinking") == Some("")
        && string(block, "signature").is_some_and(|signature| !signature.is_empty())
}

fn claude_content(block: &Value) -> Vec<PublicWireFact> {
    match string(block, "type") {
        Some("text") => string(block, "text")
            .filter(|text| !text.is_empty())
            .map_or_else(
                || diagnostic("malformedClaudeText"),
                |text| {
                    vec![PublicWireFact::Event(NormalizedProviderEvent::Output {
                        text: text.into(),
                        is_partial: false,
                    })]
                },
            ),
        Some("thinking") => string(block, "thinking")
            .filter(|text| !text.is_empty())
            .map_or_else(
                || diagnostic("malformedClaudeThinking"),
                |text| {
                    vec![PublicWireFact::Event(NormalizedProviderEvent::Thinking {
                        text: text.into(),
                        is_partial: false,
                    })]
                },
            ),
        Some("tool_use") => {
            let mut facts =
                tool_activity(block, ToolPhase::Started, "malformedClaudeToolUse", false);
            facts.extend(claude_protocol::proposed_plan(block));
            facts
        }
        _ => diagnostic("unsupportedClaudeContent"),
    }
}

fn claude_error(frame: &Value) -> Vec<PublicWireFact> {
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::ProviderFailure {
            classification: claude_protocol::failure_classification(frame),
            message: claude_protocol::failure_message(frame),
        },
    )]
}

/// A tool use Claude's own permission policy blocked before it ran. Unlike an ordinary tool
/// result, this frame carries the tool's identity directly (`tool_use_id`/`tool_name`), so no
/// runner-owned correlation is needed — it never reaches a matching `user`/`tool_result` frame.
fn permission_denied(frame: &Value) -> Vec<PublicWireFact> {
    match (string(frame, "tool_use_id"), string(frame, "tool_name")) {
        (Some(tool_use_id), Some(tool_name))
            if !tool_use_id.is_empty() && !tool_name.is_empty() =>
        {
            claude_protocol::activity(tool_use_id, tool_name, ToolPhase::Failed, None, None)
        }
        _ => diagnostic("malformedClaudePermissionDenied"),
    }
}

fn tool_activity(
    value: &Value,
    phase: ToolPhase,
    invalid: &str,
    use_type_as_name: bool,
) -> Vec<PublicWireFact> {
    let name = if use_type_as_name {
        string(value, "name").or_else(|| string(value, "type"))
    } else {
        string(value, "name")
    };
    match (string(value, "id"), name) {
        (Some(tool_use_id), Some(tool_name))
            if !tool_use_id.is_empty() && !tool_name.is_empty() =>
        {
            claude_protocol::activity(tool_use_id, tool_name, phase, None, None)
        }
        _ => diagnostic(invalid),
    }
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
