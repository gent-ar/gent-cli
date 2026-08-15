//! Declarative, pure adapter manifests for portable public protocols.

use std::collections::BTreeMap;

use gent_types::NormalizedProviderEvent;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct DeclarativeAdapterManifest {
    pub id: String,
    pub protocol_version: u16,
    /// Provider frame type to normalized event kind mapping.
    pub event_map: BTreeMap<String, String>,
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("manifest requires a non-empty id")]
    EmptyId,
    #[error("manifest maps {frame_type} to unsupported normalized event {target}")]
    UnsupportedEvent { frame_type: String, target: String },
}

impl DeclarativeAdapterManifest {
    /// Validates the portable subset before an adapter is registered.
    ///
    /// # Errors
    /// Returns an error for missing identity or unsupported event mappings.
    pub fn validate(&self) -> Result<(), ManifestError> {
        if self.id.is_empty() {
            return Err(ManifestError::EmptyId);
        }
        for (source, target) in &self.event_map {
            if !["output", "turnStarted", "turnEnded", "decisionSettled"].contains(&target.as_str())
            {
                return Err(ManifestError::UnsupportedEvent {
                    frame_type: source.clone(),
                    target: target.clone(),
                });
            }
        }
        Ok(())
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
}

fn string(frame: &Value, field: &str) -> String {
    frame
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .into()
}

#[cfg(test)]
mod tests {
    use super::DeclarativeAdapterManifest;
    use gent_types::NormalizedProviderEvent;
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
}
