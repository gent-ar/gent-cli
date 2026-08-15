use std::collections::BTreeMap;

use gent_adapters::DeclarativeAdapterManifest;
use gent_types::{NormalizedLifecycleSignal, ToolActivity, ToolPhase};
use serde_json::json;

#[test]
fn manifest_normalizes_content_safe_tool_activity() {
    let manifest = DeclarativeAdapterManifest {
        id: "fixture".into(),
        protocol_version: 1,
        event_map: BTreeMap::from([("tool".into(), "toolActivity".into())]),
    };
    assert!(manifest.validate().is_ok());
    assert_eq!(
        manifest.interpret_lifecycle(&json!({
            "type": "tool", "tool_use_id": "tool-1", "tool_name": "read_file",
            "phase": "waitingPermission", "output_digest": "sha256:abc", "input": "secret"
        })),
        Some(NormalizedLifecycleSignal::ToolActivity {
            activity: ToolActivity {
                tool_use_id: "tool-1".into(),
                tool_name: "read_file".into(),
                phase: ToolPhase::WaitingPermission,
                output_digest: Some("sha256:abc".into()),
            }
        })
    );
}

#[test]
fn incomplete_tool_activity_is_rejected() {
    let manifest = DeclarativeAdapterManifest {
        id: "fixture".into(),
        protocol_version: 1,
        event_map: BTreeMap::from([("tool".into(), "toolActivity".into())]),
    };
    assert_eq!(
        manifest.interpret_lifecycle(&json!({ "type": "tool", "phase": "started" })),
        None
    );
}
