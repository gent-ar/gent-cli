//! Codex sub-agent receipt reduction, kept apart from ordinary tool items.

use gent_types::{NormalizedProviderEvent, ToolPhase};
use serde_json::Value;

use super::super::PublicWireFact;

/// Retains only the documented child-thread to parent-tool relationship.
pub(super) fn started(item: &Value, phase: &ToolPhase) -> Vec<PublicWireFact> {
    if *phase != ToolPhase::Completed || string(item, "kind") != Some("started") {
        return Vec::new();
    }
    match (string(item, "id"), string(item, "agentThreadId")) {
        (Some(parent_tool_use_id), Some(child_id))
            if !parent_tool_use_id.is_empty() && !child_id.is_empty() =>
        {
            vec![PublicWireFact::Event(
                NormalizedProviderEvent::ChildStarted {
                    child_id: child_id.into(),
                    parent_tool_use_id: parent_tool_use_id.into(),
                },
            )]
        }
        _ => diagnostic(),
    }
}

pub(super) fn collab_started(item: &Value) -> Vec<PublicWireFact> {
    let Some(parent_tool_use_id) = string(item, "id").filter(|id| !id.is_empty()) else {
        return Vec::new();
    };
    let mut child_ids = Vec::new();
    for path in ["receiverThreadIds", "agentThreadIds"] {
        if let Some(values) = item.get(path).and_then(Value::as_array) {
            child_ids.extend(
                values
                    .iter()
                    .filter_map(Value::as_str)
                    .filter(|id| !id.is_empty()),
            );
        }
    }
    for field in ["receiverThreadId", "agentThreadId", "newThreadId"] {
        if let Some(id) = string(item, field).filter(|id| !id.is_empty()) {
            child_ids.push(id);
        }
    }
    if let Some(states) = item.get("agentsStates") {
        if let Some(states) = states.as_object() {
            child_ids.extend(
                states
                    .keys()
                    .filter(|id| !id.is_empty())
                    .map(String::as_str),
            );
        }
        if let Some(states) = states.as_array() {
            for state in states {
                for field in ["threadId", "receiverThreadId", "agentThreadId", "id"] {
                    if let Some(id) = state
                        .get(field)
                        .and_then(Value::as_str)
                        .filter(|id| !id.is_empty())
                    {
                        child_ids.push(id);
                    }
                }
            }
        }
    }
    child_ids.sort_unstable();
    child_ids.dedup();
    child_ids
        .into_iter()
        .map(|child_id| {
            PublicWireFact::Event(NormalizedProviderEvent::ChildStarted {
                child_id: child_id.into(),
                parent_tool_use_id: parent_tool_use_id.into(),
            })
        })
        .collect()
}

fn diagnostic() -> Vec<PublicWireFact> {
    vec![PublicWireFact::Event(
        NormalizedProviderEvent::TransportDiagnostic {
            classification: "malformedCodexSubAgentActivity".into(),
        },
    )]
}

fn string<'a>(value: &'a Value, field: &str) -> Option<&'a str> {
    value.get(field)?.as_str()
}
