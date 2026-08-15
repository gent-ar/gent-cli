//! Pure, documented Claude stream-JSON and Codex app-server frame normalization.

use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, ToolActivity, ToolPhase,
    TurnPhase,
};
use serde_json::Value;

use crate::PublicProvider;

/// A provider-neutral fact extracted without process, ledger, or UI access.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PublicWireFact {
    SessionStarted { provider_session_id: String },
    Event(NormalizedProviderEvent),
    Lifecycle(NormalizedLifecycleSignal),
}

/// Normalizes one documented public-provider frame. Unknown fields never enter the result.
#[must_use]
pub fn normalize_public_frame(provider: PublicProvider, frame: &Value) -> Vec<PublicWireFact> {
    match provider {
        PublicProvider::Claude => claude(frame),
        PublicProvider::Codex => codex(frame),
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
            session(frame, "malformedClaudeInit")
        }
        Some("assistant") => claude_assistant(frame),
        Some("result") => claude_result(frame),
        _ => diagnostic("unsupportedClaudeFrame"),
    }
}

fn session(frame: &Value, invalid: &str) -> Vec<PublicWireFact> {
    string(frame, "session_id")
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

fn codex(frame: &Value) -> Vec<PublicWireFact> {
    match string(frame, "method") {
        Some("thread/started") => session(
            frame.pointer("/params/thread").unwrap_or(frame),
            "malformedCodexThread",
        ),
        Some("turn/started") => turn(frame, true),
        Some("turn/completed") => turn(frame, false),
        Some("item/agentMessage/delta") => codex_delta(frame),
        Some("item/started") => codex_item(frame, ToolPhase::Started),
        Some("item/completed") => codex_item(frame, ToolPhase::Completed),
        Some(method) if method.ends_with("requestApproval") => vec![PublicWireFact::Lifecycle(
            NormalizedLifecycleSignal::AttentionRequired,
        )],
        Some(_) => diagnostic("unsupportedCodexNotification"),
        None if frame.get("error").is_some() => diagnostic("codexRpcError"),
        None => diagnostic("malformedCodexFrame"),
    }
}

fn turn(frame: &Value, started: bool) -> Vec<PublicWireFact> {
    let Some(turn) = frame.pointer("/params/turn") else {
        return diagnostic("malformedCodexTurn");
    };
    let Some(turn_id) = string(turn, "id").filter(|id| !id.is_empty()) else {
        return diagnostic("malformedCodexTurn");
    };
    if started {
        vec![PublicWireFact::Event(
            NormalizedProviderEvent::TurnStarted {
                turn_id: turn_id.into(),
            },
        )]
    } else {
        let phase = match string(turn, "status") {
            Some("completed") => TurnPhase::Ready,
            Some("interrupted") => TurnPhase::Interrupted,
            Some("failed") => TurnPhase::Failed,
            _ => return diagnostic("malformedCodexTurn"),
        };
        vec![
            PublicWireFact::Event(NormalizedProviderEvent::TurnEnded {
                turn_id: turn_id.into(),
            }),
            PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootActivity {
                activity: RootActivity::Idle,
            }),
            PublicWireFact::Lifecycle(NormalizedLifecycleSignal::RootPhase { phase }),
        ]
    }
}

fn codex_delta(frame: &Value) -> Vec<PublicWireFact> {
    frame
        .pointer("/params/delta")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map_or_else(
            || diagnostic("malformedCodexMessageDelta"),
            |text| {
                vec![PublicWireFact::Event(NormalizedProviderEvent::Output {
                    text: text.into(),
                })]
            },
        )
}

fn codex_item(frame: &Value, phase: ToolPhase) -> Vec<PublicWireFact> {
    let item = frame.pointer("/params/item").unwrap_or(frame);
    let Some(kind) = string(item, "type") else {
        return diagnostic("malformedCodexItem");
    };
    if !matches!(kind, "commandExecution" | "fileChange" | "mcpToolCall") {
        return Vec::new();
    }
    tool_activity(item, phase, "malformedCodexItem", true)
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
