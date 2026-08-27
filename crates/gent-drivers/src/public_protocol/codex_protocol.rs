use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, ToolActivity, ToolPhase};
use serde_json::Value;

use super::{PublicCompactionObservation, PublicWireFact};

mod output;
mod plan;
mod stream;
mod subagent;
#[path = "codex_protocol_support.rs"]
mod support;
mod terminal;
use plan::updated;
use support::{attention, diagnostic, housekeeping, inert_item, string, tool_kind};

pub(super) fn normalize(frame: &Value) -> Vec<PublicWireFact> {
    match string(frame, "method") {
        Some("thread/started") => thread_started(frame),
        Some("turn/started") => terminal::started(frame),
        Some("turn/completed") => terminal::completed(frame),
        Some("turn/aborted") => terminal::aborted(frame),
        Some("turn/failed") => terminal::failed(frame),
        Some("item/agentMessage/delta") => agent_message_delta(frame),
        Some(
            "item/reasoning/textDelta" | "item/reasoning/summaryTextDelta" | "item/plan/delta",
        ) => thinking_delta(frame),
        Some("item/reasoning/summaryPartAdded" | "thread/status/changed") => Vec::new(),
        Some(
            "item/commandExecution/outputDelta"
            | "item/fileChange/outputDelta"
            | "item/fileChange/patchUpdated"
            | "item/commandExecution/terminalInteraction",
        ) => stream::tool_output(frame),
        Some("item/started") => item(frame, ToolPhase::Started),
        Some("item/completed") => completed_item(frame),
        Some("item/mcpToolCall/progress") => stream::mcp_progress(frame),
        Some("turn/plan/updated") => updated(frame),
        Some("thread/compacted") => vec![PublicWireFact::Compaction(
            PublicCompactionObservation::Completed,
        )],
        Some("error" | "codex/event/stream_error" | "codex/event/error") => stream::error(frame),
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
        Some("subAgentActivity") => subagent::started(item, &phase),
        Some("collabAgentToolCall" | "collabToolCall") => {
            let is_start = phase == ToolPhase::Started;
            let mut facts = tool_activity(item, phase);
            if is_start {
                facts.extend(subagent::collab_started(item));
            }
            facts
        }
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
    let mut facts = item(frame, phase);
    if matches!(
        string(completed, "type"),
        Some("fileChange" | "mcpToolCall" | "dynamicToolCall")
    ) {
        facts.extend(output::completed_tool_output(completed));
    }
    facts
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
    match (string(item, "id"), tool_name(item)) {
        (Some(tool_use_id), Some(tool_name))
            if !tool_use_id.is_empty() && !tool_name.is_empty() =>
        {
            vec![PublicWireFact::Lifecycle(
                NormalizedLifecycleSignal::ToolActivity {
                    activity: ToolActivity {
                        tool_use_id: tool_use_id.into(),
                        tool_name,
                        phase,
                        output_digest: None,
                    },
                },
            )]
        }
        _ => diagnostic("malformedCodexItem"),
    }
}

fn tool_name(item: &Value) -> Option<String> {
    if string(item, "type") == Some("mcpToolCall") {
        let server = string(item, "server").filter(|value| !value.is_empty());
        let tool = string(item, "tool").filter(|value| !value.is_empty());
        if let (Some(server), Some(tool)) = (server, tool) {
            return Some(format!("mcp__{}__{tool}", server.replace('-', "_")));
        }
    }
    string(item, "name")
        .or_else(|| string(item, "type"))
        .map(str::to_owned)
}

fn thinking_delta(frame: &Value) -> Vec<PublicWireFact> {
    frame
        .pointer("/params/delta")
        .and_then(Value::as_str)
        .filter(|text| !text.is_empty())
        .map_or_else(
            || diagnostic("malformedCodexThinkingDelta"),
            |text| {
                vec![PublicWireFact::Event(NormalizedProviderEvent::Thinking {
                    text: text.into(),
                    is_partial: true,
                })]
            },
        )
}
