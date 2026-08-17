//! Validation nested below the manifest domain boundary.

use super::DeclarativeAdapterManifest;

const SUPPORTED_PROTOCOL_VERSION: u16 = 1;

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum ManifestError {
    #[error("manifest requires a non-empty id")]
    EmptyId,
    #[error("manifest protocol version {actual} is unsupported; expected {expected}")]
    UnsupportedProtocol { actual: u16, expected: u16 },
    #[error("manifest has an empty provider frame type")]
    EmptyFrameType,
    #[error("manifest maps {frame_type} to unsupported normalized event {target}")]
    UnsupportedEvent { frame_type: String, target: String },
}

pub(super) fn validate(manifest: &DeclarativeAdapterManifest) -> Result<(), ManifestError> {
    if manifest.id.is_empty() {
        return Err(ManifestError::EmptyId);
    }
    if manifest.protocol_version != SUPPORTED_PROTOCOL_VERSION {
        return Err(ManifestError::UnsupportedProtocol {
            actual: manifest.protocol_version,
            expected: SUPPORTED_PROTOCOL_VERSION,
        });
    }
    for (frame_type, target) in &manifest.event_map {
        if frame_type.is_empty() {
            return Err(ManifestError::EmptyFrameType);
        }
        if !is_supported_target(target) {
            return Err(ManifestError::UnsupportedEvent {
                frame_type: frame_type.clone(),
                target: target.clone(),
            });
        }
    }
    Ok(())
}

fn is_supported_target(target: &str) -> bool {
    matches!(
        target,
        "output"
            | "turnStarted"
            | "turnEnded"
            | "decisionSettled"
            | "rootPhase"
            | "rootActivity"
            | "childPhase"
            | "commandPhase"
            | "toolActivity"
            | "attentionRequired"
            | "attentionCleared"
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::{ManifestError, validate};
    use crate::manifest::DeclarativeAdapterManifest;

    fn manifest() -> DeclarativeAdapterManifest {
        DeclarativeAdapterManifest {
            id: "fixture".into(),
            protocol_version: 1,
            event_map: BTreeMap::from([("text".into(), "output".into())]),
        }
    }

    #[test]
    fn rejects_missing_identity_unsupported_schema_and_empty_frame_types() {
        let mut candidate = manifest();
        candidate.id.clear();
        assert_eq!(validate(&candidate), Err(ManifestError::EmptyId));
        candidate = manifest();
        candidate.protocol_version = 2;
        assert!(matches!(
            validate(&candidate),
            Err(ManifestError::UnsupportedProtocol { .. })
        ));
        candidate = manifest();
        candidate.event_map = BTreeMap::from([(String::new(), "output".into())]);
        assert_eq!(validate(&candidate), Err(ManifestError::EmptyFrameType));
    }

    #[test]
    fn rejects_unknown_normalized_targets() {
        let mut candidate = manifest();
        candidate.event_map = BTreeMap::from([("text".into(), "rawProviderEvent".into())]);
        assert!(matches!(
            validate(&candidate),
            Err(ManifestError::UnsupportedEvent { .. })
        ));
    }
}
