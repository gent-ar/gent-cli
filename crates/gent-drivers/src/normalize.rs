//! Pure normalization of supported public-driver frames; no process or persistence access.

use gent_types::{NormalizedProviderEvent, WorkPhase};
use serde_json::Value;

/// Converts known provider-neutral fields into the stable event contract.
/// Unknown frames are preserved as non-mutating diagnostics for bounded monitoring.
#[must_use]
pub fn normalize(frame: &Value) -> NormalizedProviderEvent {
    let kind = field(frame, "type");
    match kind {
        Some("output") => NormalizedProviderEvent::Output {
            text: field(frame, "text").unwrap_or_default().into(),
        },
        Some("turn_started") => NormalizedProviderEvent::TurnStarted {
            turn_id: field(frame, "turn_id").unwrap_or_default().into(),
        },
        Some("turn_ended") => NormalizedProviderEvent::TurnEnded {
            turn_id: field(frame, "turn_id").unwrap_or_default().into(),
        },
        Some("child_started") => NormalizedProviderEvent::ChildStarted {
            child_id: field(frame, "child_id").unwrap_or_default().into(),
            parent_tool_use_id: field(frame, "parent_tool_use_id")
                .unwrap_or_default()
                .into(),
        },
        Some("child_terminal") => NormalizedProviderEvent::ChildTerminal {
            child_id: field(frame, "child_id").unwrap_or_default().into(),
            phase: work_phase(field(frame, "phase")),
        },
        Some("command_terminal") => NormalizedProviderEvent::CommandTerminal {
            command_id: field(frame, "command_id").unwrap_or_default().into(),
            phase: work_phase(field(frame, "phase")),
        },
        Some("decision_settled") => NormalizedProviderEvent::DecisionSettled {
            decision_id: field(frame, "decision_id").unwrap_or_default().into(),
        },
        _ => NormalizedProviderEvent::TransportDiagnostic {
            classification: "unknownProviderFrame".into(),
        },
    }
}

fn field<'a>(frame: &'a Value, name: &str) -> Option<&'a str> {
    frame.get(name)?.as_str()
}

fn work_phase(value: Option<&str>) -> WorkPhase {
    match value {
        Some("done") => WorkPhase::Done,
        Some("failed") => WorkPhase::Failed,
        Some("interrupted") => WorkPhase::Interrupted,
        Some("waiting_permission") => WorkPhase::WaitingPermission,
        Some("pending") => WorkPhase::Pending,
        _ => WorkPhase::Running,
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{NormalizedProviderEvent, WorkPhase};
    use serde_json::json;

    use super::normalize;

    #[test]
    fn unknown_frames_are_diagnostics_not_lifecycle_events() {
        assert_eq!(
            normalize(&json!({ "type": "future/frame", "data": 1 })),
            NormalizedProviderEvent::TransportDiagnostic {
                classification: "unknownProviderFrame".into()
            }
        );
    }

    #[test]
    fn child_terminal_preserves_its_terminal_phase() {
        assert_eq!(
            normalize(&json!({ "type": "child_terminal", "child_id": "child", "phase": "done" })),
            NormalizedProviderEvent::ChildTerminal {
                child_id: "child".into(),
                phase: WorkPhase::Done
            }
        );
    }
}
