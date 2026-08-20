//! Secret-free normalization of documented Codex app-server notifications.

use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, ToolActivity, ToolPhase,
    TurnPhase,
};
use serde_json::Value;

use super::{PublicCompactionObservation, PublicWireFact};

mod subagent;

/// Reduces one current Codex app-server frame without retaining its raw payload.
pub(super) fn normalize(frame: &Value) -> Vec<PublicWireFact> {
    match string(frame, "method") {
        Some("thread/started") => thread_started(frame),
        Some("turn/started") => turn_started(frame),
        Some("turn/completed") => completed_turn(frame),
        Some("turn/failed") => legacy_terminal(frame, TurnPhase::Failed),
        Some("turn/aborted") => legacy_terminal(frame, TurnPhase::Interrupted),
        Some("item/agentMessage/delta") => agent_message_delta(frame),
        Some("item/started") => item(frame, ToolPhase::Started),
        Some("item/completed") => completed_item(frame),
        Some("turn/plan/updated") => plan_update(frame),
        Some("item/plan/delta") => plan_delta(frame),
        Some("thread/compacted") => diagnostic("codexContextCompacted"),
        // The turn bridge separately recognizes terminal child status only after an
        // explicit launch correlation. A root-thread status is otherwise inert.
        Some("thread/status/changed") => Vec::new(),
        Some(method) if method.ends_with("requestApproval") => attention(),
        Some(method) if housekeeping(method) => Vec::new(),
        Some(_) => diagnostic("unsupportedCodexNotification"),
        None if frame.get("error").is_some() => diagnostic("codexRpcError"),
        None => diagnostic("malformedCodexFrame"),
    }
}

fn thread_started(frame: &Value) -> Vec<PublicWireFact> {
    let id = frame.pointer("/params/thread/id").and_then(Value::as_str);
    id.filter(|value| !value.is_empty()).map_or_else(
        || diagnostic("malformedCodexThread"),
        |provider_session_id| {
            vec![PublicWireFact::SessionStarted {
                provider_session_id: provider_session_id.into(),
            }]
        },
    )
}

fn turn_started(frame: &Value) -> Vec<PublicWireFact> {
    turn_id(frame).map_or_else(
        || diagnostic("malformedCodexTurn"),
        |turn_id| {
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::TurnStarted {
                    turn_id: turn_id.into(),
                },
            )]
        },
    )
}

fn completed_turn(frame: &Value) -> Vec<PublicWireFact> {
    let Some(turn) = frame.pointer("/params/turn") else {
        return diagnostic("malformedCodexTurn");
    };
    let phase = match string(turn, "status") {
        Some("completed") => TurnPhase::Ready,
        Some("interrupted") => TurnPhase::Interrupted,
        Some("failed") => TurnPhase::Failed,
        _ => return diagnostic("malformedCodexTurn"),
    };
    terminal(turn_id(frame), phase)
}

fn legacy_terminal(frame: &Value, phase: TurnPhase) -> Vec<PublicWireFact> {
    terminal(turn_id(frame), phase)
}

fn terminal(turn_id: Option<&str>, phase: TurnPhase) -> Vec<PublicWireFact> {
    let Some(turn_id) = turn_id else {
        return diagnostic("malformedCodexTurn");
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

fn turn_id(frame: &Value) -> Option<&str> {
    frame
        .pointer("/params/turn/id")
        .or_else(|| frame.pointer("/params/turnId"))
        .and_then(Value::as_str)
        .filter(|id| !id.is_empty())
}

fn agent_message_delta(frame: &Value) -> Vec<PublicWireFact> {
    frame
        .pointer("/params/delta")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map_or_else(
            || diagnostic("malformedCodexMessageDelta"),
            |text| {
                vec![PublicWireFact::Event(NormalizedProviderEvent::Output {
                    text: text.into(),
                    is_partial: true,
                })]
            },
        )
}

fn item(frame: &Value, phase: ToolPhase) -> Vec<PublicWireFact> {
    let Some(item) = frame.pointer("/params/item") else {
        return diagnostic("malformedCodexItem");
    };
    match string(item, "type") {
        Some("contextCompaction") => compaction(item, &phase),
        // Current Codex app-server versions publish a child launch as a completed
        // `subAgentActivity { kind: "started" }` receipt. The item id is the
        // parent Agent tool and `agentThreadId` is the child identity; retain only
        // that proven relationship rather than treating the receipt as a completed
        // generic tool.
        Some("subAgentActivity") => subagent::started(item, phase),
        Some(kind) if tool_kind(kind) => tool_activity(item, phase),
        Some(kind) if inert_item(kind) => Vec::new(),
        Some(_) => diagnostic("unsupportedCodexItem"),
        None => diagnostic("malformedCodexItem"),
    }
}

fn compaction(item: &Value, phase: &ToolPhase) -> Vec<PublicWireFact> {
    let observation = match phase {
        ToolPhase::Started => PublicCompactionObservation::Started,
        ToolPhase::WaitingPermission => return diagnostic("unsupportedCodexCompactionPhase"),
        ToolPhase::Completed => PublicCompactionObservation::Completed,
        ToolPhase::Failed => PublicCompactionObservation::Failed {
            // The documented item status has no structured reason field. Do not infer
            // `tooFewGroups` from text until a recorded provider contract proves one.
            failure: gent_types::AgentChatCompactionFailure::ProviderFailed,
        },
    };
    if *phase == ToolPhase::Failed && string(item, "status") != Some("failed") {
        return diagnostic("malformedCodexCompaction");
    }
    vec![PublicWireFact::Compaction(observation)]
}

fn completed_item(frame: &Value) -> Vec<PublicWireFact> {
    let Some(completed) = frame.pointer("/params/item") else {
        return diagnostic("malformedCodexItem");
    };
    if string(completed, "type") == Some("agentMessage") {
        return completed_agent_message(completed);
    }
    let phase = match frame.pointer("/params/item/status").and_then(Value::as_str) {
        Some("failed") => ToolPhase::Failed,
        _ => ToolPhase::Completed,
    };
    item(frame, phase)
}

fn completed_agent_message(item: &Value) -> Vec<PublicWireFact> {
    string(item, "text")
        .filter(|text| !text.is_empty())
        .map_or_else(
            || diagnostic("malformedCodexAgentMessage"),
            |text| {
                vec![PublicWireFact::Event(NormalizedProviderEvent::Output {
                    text: text.into(),
                    is_partial: false,
                })]
            },
        )
}

fn tool_activity(item: &Value, phase: ToolPhase) -> Vec<PublicWireFact> {
    match (
        string(item, "id"),
        string(item, "name").or_else(|| string(item, "type")),
    ) {
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
        _ => diagnostic("malformedCodexItem"),
    }
}

fn plan_update(frame: &Value) -> Vec<PublicWireFact> {
    let valid = frame
        .pointer("/params/plan")
        .and_then(Value::as_array)
        .is_some_and(|steps| {
            !steps.is_empty()
                && steps
                    .iter()
                    .any(|step| string(step, "step").is_some_and(|text| !text.is_empty()))
        });
    if valid {
        diagnostic("codexPlanUpdated")
    } else {
        diagnostic("malformedCodexPlanUpdate")
    }
}

fn plan_delta(frame: &Value) -> Vec<PublicWireFact> {
    if frame
        .pointer("/params/delta")
        .and_then(Value::as_str)
        .is_some_and(|text| !text.is_empty())
    {
        diagnostic("codexPlanDelta")
    } else {
        diagnostic("malformedCodexPlanDelta")
    }
}

fn attention() -> Vec<PublicWireFact> {
    vec![PublicWireFact::Lifecycle(
        NormalizedLifecycleSignal::AttentionRequired,
    )]
}

fn tool_kind(kind: &str) -> bool {
    matches!(
        kind,
        "commandExecution"
            | "fileChange"
            | "mcpToolCall"
            | "dynamicToolCall"
            | "collabAgentToolCall"
            | "collabToolCall"
            | "webSearch"
            | "imageView"
            | "imageGeneration"
            | "hookPrompt"
    )
}

fn inert_item(kind: &str) -> bool {
    matches!(
        kind,
        "agentMessage"
            | "userMessage"
            | "reasoning"
            | "plan"
            | "enteredReviewMode"
            | "exitedReviewMode"
            | "contextCompaction"
    )
}

fn housekeeping(method: &str) -> bool {
    matches!(
        method,
        "thread/settings/updated"
            | "thread/tokenUsage/updated"
            | "thread/name/updated"
            | "turn/diff/updated"
            | "account/rateLimits/updated"
            | "skills/changed"
    )
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
