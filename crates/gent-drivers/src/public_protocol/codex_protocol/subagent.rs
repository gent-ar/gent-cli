use gent_types::{NormalizedProviderEvent, ToolPhase};
use serde_json::Value;

use super::super::PublicWireFact;

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
    let mut child_ids = item
        .get("receiverThreadIds")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .chain(
            item.get("agentsStates")
                .and_then(Value::as_object)
                .into_iter()
                .flat_map(|states| states.keys().map(String::as_str)),
        )
        .filter(|id| !id.is_empty())
        .collect::<Vec<_>>();
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
