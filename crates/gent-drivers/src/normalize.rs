//! Pure normalization of supported public-driver frames; no process or persistence access.

use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, ToolActivity, ToolPhase,
    TurnPhase, WorkPhase,
};
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
        Some("root_activity") => root_activity(field(frame, "activity"))
            .map(|activity| NormalizedLifecycleSignal::RootActivity { activity }),
        Some("child_phase") => Some(NormalizedLifecycleSignal::ChildPhase {
            child_id: field(frame, "child_id")?.into(),
            phase: work_phase(field(frame, "phase")),
        }),
        Some("command_phase") => Some(NormalizedLifecycleSignal::CommandPhase {
            command_id: field(frame, "command_id")?.into(),
            phase: work_phase(field(frame, "phase")),
        }),
        Some("tool_activity") => Some(NormalizedLifecycleSignal::ToolActivity {
            activity: ToolActivity {
                tool_use_id: field(frame, "tool_use_id")?.into(),
                tool_name: field(frame, "tool_name")?.into(),
                phase: tool_phase(field(frame, "phase"))?,
                output_digest: field(frame, "output_digest").map(Into::into),
            },
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

fn root_activity(value: Option<&str>) -> Option<RootActivity> {
    match value {
        Some("generating") => Some(RootActivity::Generating),
        Some("waiting") => Some(RootActivity::Waiting),
        Some("idle") => Some(RootActivity::Idle),
        _ => None,
    }
}

fn tool_phase(value: Option<&str>) -> Option<ToolPhase> {
    match value {
        Some("started") => Some(ToolPhase::Started),
        Some("waiting_permission") => Some(ToolPhase::WaitingPermission),
        Some("completed") => Some(ToolPhase::Completed),
        Some("failed") => Some(ToolPhase::Failed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use gent_types::{
        NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, ToolActivity, ToolPhase,
        TurnPhase, WorkPhase,
    };
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
        assert_eq!(
            normalize_lifecycle(&json!({ "type": "root_activity", "activity": "generating" })),
            Some(NormalizedLifecycleSignal::RootActivity {
                activity: RootActivity::Generating
            })
        );
    }

    #[test]
    fn lifecycle_normalization_covers_work_and_every_root_phase() {
        for (raw, phase) in [
            ("processing", TurnPhase::Processing),
            ("waiting_permission", TurnPhase::WaitingPermission),
            ("waiting_question", TurnPhase::WaitingQuestion),
            ("ready", TurnPhase::Ready),
            ("interrupted", TurnPhase::Interrupted),
            ("dead", TurnPhase::Dead),
            ("failed", TurnPhase::Failed),
        ] {
            assert_eq!(
                normalize_lifecycle(&json!({ "type": "root_phase", "phase": raw })),
                Some(NormalizedLifecycleSignal::RootPhase { phase })
            );
        }
        assert_eq!(
            normalize_lifecycle(
                &json!({ "type": "child_phase", "child_id": "child", "phase": "done" })
            ),
            Some(NormalizedLifecycleSignal::ChildPhase {
                child_id: "child".into(),
                phase: WorkPhase::Done
            })
        );
        assert_eq!(
            normalize_lifecycle(
                &json!({ "type": "command_phase", "command_id": "command", "phase": "waiting_permission" })
            ),
            Some(NormalizedLifecycleSignal::CommandPhase {
                command_id: "command".into(),
                phase: WorkPhase::WaitingPermission
            })
        );
        assert_eq!(
            normalize_lifecycle(&json!({ "type": "decision_settled" })),
            Some(NormalizedLifecycleSignal::AttentionCleared)
        );
        assert_eq!(
            normalize_lifecycle(&json!({ "type": "root_phase", "phase": "future" })),
            None
        );
        assert_eq!(
            normalize_lifecycle(&json!({ "type": "root_activity", "activity": "future" })),
            None
        );
        assert_eq!(normalize_lifecycle(&json!({ "type": "child_phase" })), None);
        assert_eq!(
            normalize_lifecycle(&json!({
                "type": "tool_activity", "tool_use_id": "tool-1", "tool_name": "read_file",
                "phase": "completed", "output_digest": "sha256:abc"
            })),
            Some(NormalizedLifecycleSignal::ToolActivity {
                activity: ToolActivity {
                    tool_use_id: "tool-1".into(),
                    tool_name: "read_file".into(),
                    phase: ToolPhase::Completed,
                    output_digest: Some("sha256:abc".into()),
                }
            })
        );
    }
}
