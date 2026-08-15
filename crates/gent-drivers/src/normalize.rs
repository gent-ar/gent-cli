//! Pure normalization of supported public-driver frames; no process or persistence access.

use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, TurnPhase, WorkPhase};
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

/// Converts an explicit lifecycle frame into an additive status signal.
///
/// This is separate from [`normalize`] so existing consumers retain their stable content-event
/// contract while capable callers can persist richer status facts.
#[must_use]
pub fn normalize_lifecycle(frame: &Value) -> Option<NormalizedLifecycleSignal> {
    match field(frame, "type") {
        Some("root_phase") => root_phase(field(frame, "phase"))
            .map(|phase| NormalizedLifecycleSignal::RootPhase { phase }),
        Some("child_phase") => Some(NormalizedLifecycleSignal::ChildPhase {
            child_id: field(frame, "child_id")?.into(),
            phase: work_phase(field(frame, "phase")),
        }),
        Some("command_phase") => Some(NormalizedLifecycleSignal::CommandPhase {
            command_id: field(frame, "command_id")?.into(),
            phase: work_phase(field(frame, "phase")),
        }),
        Some("decision_requested") => Some(NormalizedLifecycleSignal::AttentionRequired),
        Some("decision_settled") => Some(NormalizedLifecycleSignal::AttentionCleared),
        _ => None,
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

fn root_phase(value: Option<&str>) -> Option<TurnPhase> {
    match value {
        Some("processing") => Some(TurnPhase::Processing),
        Some("waiting_permission") => Some(TurnPhase::WaitingPermission),
        Some("waiting_question") => Some(TurnPhase::WaitingQuestion),
        Some("compacting") => Some(TurnPhase::Compacting),
        Some("ready") => Some(TurnPhase::Ready),
        Some("interrupted") => Some(TurnPhase::Interrupted),
        Some("dead") => Some(TurnPhase::Dead),
        Some("failed") => Some(TurnPhase::Failed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, TurnPhase, WorkPhase};
    use serde_json::json;

    use super::{normalize, normalize_lifecycle};

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

    #[test]
    fn lifecycle_frames_preserve_waiting_and_attention() {
        assert_eq!(
            normalize_lifecycle(&json!({ "type": "root_phase", "phase": "compacting" })),
            Some(NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Compacting
            })
        );
        assert_eq!(
            normalize_lifecycle(&json!({ "type": "decision_requested" })),
            Some(NormalizedLifecycleSignal::AttentionRequired)
        );
    }
}
