//! Declarative, pure adapter manifests for portable public protocols.

use std::collections::BTreeMap;

use gent_types::{
    NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, ToolActivity, ToolPhase,
    TurnPhase, WorkPhase,
};
use serde::{Deserialize, Serialize};

pub use crate::manifest_validation::ManifestError;
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeclarativeAdapterManifest {
    pub id: String,
    pub protocol_version: u16,
    /// Provider frame type to normalized event kind mapping.
    pub event_map: BTreeMap<String, String>,
}

impl DeclarativeAdapterManifest {
    /// Validates the portable subset before an adapter is registered.
    ///
    /// # Errors
    /// Returns an error for missing identity or unsupported event mappings.
    pub fn validate(&self) -> Result<(), ManifestError> {
        crate::manifest_validation::validate(self)
    }

    /// Interprets a provider frame without process, persistence, or product-policy access.
    #[must_use]
    pub fn interpret(&self, frame: &Value) -> NormalizedProviderEvent {
        let source = frame
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default();
        match self.event_map.get(source).map(String::as_str) {
            Some("output") => NormalizedProviderEvent::Output {
                text: string(frame, "text"),
            },
            Some("turnStarted") => NormalizedProviderEvent::TurnStarted {
                turn_id: string(frame, "turn_id"),
            },
            Some("turnEnded") => NormalizedProviderEvent::TurnEnded {
                turn_id: string(frame, "turn_id"),
            },
            Some("decisionSettled") => NormalizedProviderEvent::DecisionSettled {
                decision_id: string(frame, "decision_id"),
            },
            _ => NormalizedProviderEvent::TransportDiagnostic {
                classification: "unmappedAdapterFrame".into(),
            },
        }
    }

    /// Interprets declared lifecycle facts without changing the content-event API.
    #[must_use]
    pub fn interpret_lifecycle(&self, frame: &Value) -> Option<NormalizedLifecycleSignal> {
        match self.target(frame)? {
            "rootPhase" => Some(NormalizedLifecycleSignal::RootPhase {
                phase: root_phase(&string(frame, "phase"))?,
            }),
            "rootActivity" => Some(NormalizedLifecycleSignal::RootActivity {
                activity: root_activity(&string(frame, "activity"))?,
            }),
            "childPhase" => Some(NormalizedLifecycleSignal::ChildPhase {
                child_id: non_empty(frame, "child_id")?,
                phase: work_phase(&string(frame, "phase")),
            }),
            "commandPhase" => Some(NormalizedLifecycleSignal::CommandPhase {
                command_id: non_empty(frame, "command_id")?,
                phase: work_phase(&string(frame, "phase")),
            }),
            "toolActivity" => Some(NormalizedLifecycleSignal::ToolActivity {
                activity: ToolActivity {
                    tool_use_id: non_empty(frame, "tool_use_id")?,
                    tool_name: non_empty(frame, "tool_name")?,
                    phase: tool_phase(&string(frame, "phase"))?,
                    output_digest: non_empty(frame, "output_digest"),
                },
            }),
            "attentionRequired" => Some(NormalizedLifecycleSignal::AttentionRequired),
            "attentionCleared" => Some(NormalizedLifecycleSignal::AttentionCleared),
            _ => None,
        }
    }

    fn target<'a>(&'a self, frame: &Value) -> Option<&'a str> {
        self.event_map
            .get(frame.get("type")?.as_str()?)
            .map(String::as_str)
    }
}

fn string(frame: &Value, field: &str) -> String {
    frame
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}

fn non_empty(frame: &Value, field: &str) -> Option<String> {
    let value = string(frame, field);
    (!value.is_empty()).then_some(value)
}

fn work_phase(value: &str) -> WorkPhase {
    match value {
        "pending" => WorkPhase::Pending,
        "waitingPermission" => WorkPhase::WaitingPermission,
        "done" => WorkPhase::Done,
        "failed" => WorkPhase::Failed,
        "interrupted" => WorkPhase::Interrupted,
        _ => WorkPhase::Running,
    }
}

fn root_phase(value: &str) -> Option<TurnPhase> {
    match value {
        "processing" => Some(TurnPhase::Processing),
        "waitingPermission" => Some(TurnPhase::WaitingPermission),
        "waitingQuestion" => Some(TurnPhase::WaitingQuestion),
        "compacting" => Some(TurnPhase::Compacting),
        "ready" => Some(TurnPhase::Ready),
        "interrupted" => Some(TurnPhase::Interrupted),
        "dead" => Some(TurnPhase::Dead),
        "failed" => Some(TurnPhase::Failed),
        _ => None,
    }
}

fn root_activity(value: &str) -> Option<RootActivity> {
    match value {
        "generating" => Some(RootActivity::Generating),
        "waiting" => Some(RootActivity::Waiting),
        "idle" => Some(RootActivity::Idle),
        _ => None,
    }
}

fn tool_phase(value: &str) -> Option<ToolPhase> {
    match value {
        "started" => Some(ToolPhase::Started),
        "waitingPermission" => Some(ToolPhase::WaitingPermission),
        "completed" => Some(ToolPhase::Completed),
        "failed" => Some(ToolPhase::Failed),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::DeclarativeAdapterManifest;
    use gent_types::{NormalizedLifecycleSignal, NormalizedProviderEvent, RootActivity, TurnPhase};
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test]
    fn manifest_interprets_only_declared_public_frames() {
        let manifest = DeclarativeAdapterManifest {
            id: "fixture".into(),
            protocol_version: 1,
            event_map: BTreeMap::from([("text".into(), "output".into())]),
        };
        assert!(manifest.validate().is_ok());
        assert_eq!(
            manifest.interpret(&json!({ "type": "text", "text": "hi" })),
            NormalizedProviderEvent::Output { text: "hi".into() }
        );
        assert!(matches!(
            manifest.interpret(&json!({ "type": "other" })),
            NormalizedProviderEvent::TransportDiagnostic { .. }
        ));
    }

    #[test]
    fn manifest_interprets_declared_lifecycle_facts() {
        let manifest = DeclarativeAdapterManifest {
            id: "fixture".into(),
            protocol_version: 1,
            event_map: BTreeMap::from([("state".into(), "rootPhase".into())]),
        };
        assert_eq!(
            manifest.interpret_lifecycle(&json!({ "type": "state", "phase": "compacting" })),
            Some(NormalizedLifecycleSignal::RootPhase {
                phase: TurnPhase::Compacting
            })
        );
    }

    #[test]
    fn manifest_interprets_work_attention_and_rejects_invalid_lifecycle_frames() {
        let manifest = DeclarativeAdapterManifest {
            id: "fixture".into(),
            protocol_version: 1,
            event_map: BTreeMap::from([
                ("activity".into(), "rootActivity".into()),
                ("child".into(), "childPhase".into()),
                ("command".into(), "commandPhase".into()),
                ("need".into(), "attentionRequired".into()),
                ("clear".into(), "attentionCleared".into()),
            ]),
        };
        assert_eq!(
            manifest.interpret_lifecycle(&json!({ "type": "activity", "activity": "generating" })),
            Some(NormalizedLifecycleSignal::RootActivity {
                activity: RootActivity::Generating
            })
        );
        assert_eq!(
            manifest
                .interpret_lifecycle(&json!({ "type": "child", "child_id": "a", "phase": "done" })),
            Some(NormalizedLifecycleSignal::ChildPhase {
                child_id: "a".into(),
                phase: gent_types::WorkPhase::Done
            })
        );
        assert_eq!(
            manifest.interpret_lifecycle(
                &json!({ "type": "command", "command_id": "b", "phase": "waitingPermission" })
            ),
            Some(NormalizedLifecycleSignal::CommandPhase {
                command_id: "b".into(),
                phase: gent_types::WorkPhase::WaitingPermission
            })
        );
        assert_eq!(
            manifest.interpret_lifecycle(&json!({ "type": "need" })),
            Some(NormalizedLifecycleSignal::AttentionRequired)
        );
        assert_eq!(
            manifest.interpret_lifecycle(&json!({ "type": "clear" })),
            Some(NormalizedLifecycleSignal::AttentionCleared)
        );
        assert_eq!(
            manifest.interpret_lifecycle(&json!({ "type": "child", "phase": "done" })),
            None
        );
    }
}
