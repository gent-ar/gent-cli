//! Pure, documented Claude stream-JSON and Codex app-server frame normalization.

use gent_types::{
    AgentChatCompactionFailure, NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity,
    ToolActivity, ToolPhase, TurnPhase,
};
use serde_json::Value;

use crate::PublicProvider;

mod claude_protocol;
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
    match string(frame, "type") {
        Some("system") if string(frame, "subtype") == Some("init") => {
            session(frame, "session_id", "malformedClaudeInit")
        }
        Some("stream_event") => claude_stream_event(frame),
        Some("assistant") => claude_assistant(frame),
        Some("user") => claude_protocol::user(frame),
        Some("system")
            if matches!(
                string(frame, "subtype"),
                Some("task_started" | "task_progress")
            ) =>
        {
            claude_protocol::background_activity(frame)
        }
        Some("result") => claude_result(frame),
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
        Some("message_start") => vec![PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::RootActivity {
                activity: RootActivity::Generating,
            },
        )],
        Some("content_block_start") => claude_stream_block_start(event),
        Some("content_block_delta") => claude_stream_delta(event),
        Some("content_block_stop")
        | Some("message_stop")
        | Some("message_delta")
        | Some("ping") => Vec::new(),
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
        Some("text") | Some("thinking") => Vec::new(),
        Some(_) => diagnostic("unsupportedClaudeContentBlock"),
        None => diagnostic("malformedClaudeContentBlockStart"),
    }
}

fn claude_stream_delta(event: &Value) -> Vec<PublicWireFact> {
    let Some(delta) = event.get("delta") else {
        return diagnostic("malformedClaudeContentBlockDelta");
    };
    match string(delta, "type") {
        Some("text_delta") => partial_text(delta, "text", false),
        Some("thinking_delta") => partial_text(delta, "thinking", true),
        // Claude supplies only a content-block index here. Associating it with a tool ID needs
        // runner-owned block state, so a frame-local reducer must not guess.
        Some("input_json_delta") => Vec::new(),
        Some(_) => diagnostic("unsupportedClaudeContentBlockDelta"),
        None => diagnostic("malformedClaudeContentBlockDelta"),
    }
}

fn partial_text(delta: &Value, field: &str, thinking: bool) -> Vec<PublicWireFact> {
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
    let facts: Vec<_> = content.iter().flat_map(claude_content).collect();
    if facts.is_empty() {
        diagnostic("emptyClaudeAssistant")
    } else {
        facts
    }
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
        Some("tool_use") => {
            tool_activity(block, ToolPhase::Started, "malformedClaudeToolUse", false)
        }
        _ => diagnostic("unsupportedClaudeContent"),
    }
}

fn claude_result(frame: &Value) -> Vec<PublicWireFact> {
    let failed = frame
        .get("is_error")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let phase = if failed {
        TurnPhase::Failed
    } else {
        TurnPhase::Ready
    };
    vec![
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootActivity {
            activity: RootActivity::Idle,
        }),
        PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase { phase }),
    ]
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
            vec![PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ToolActivity {
                    activity: ToolActivity {
                        tool_use_id: tool_use_id.into(),
                        tool_name: tool_name.into(),
                        phase,
                        output_digest: None,
                    },
                },
            )]
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
