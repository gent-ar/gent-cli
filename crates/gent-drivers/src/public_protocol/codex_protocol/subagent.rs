//! Codex sub-agent receipt reduction, kept apart from ordinary tool items.

use gent_types::{NormalizedProviderEvent, ToolPhase};
use serde_json::Value;

use super::super::PublicWireFact;

/// Retains only the documented child-thread to parent-tool relationship.
pub(super) fn started(item: &Value, phase: ToolPhase) -> Vec<PublicWireFact> {
    if phase != ToolPhase::Completed || string(item, "kind") != Some("started") {
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
