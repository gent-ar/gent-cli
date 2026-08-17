//! Negotiated local IPC for durable provider-neutral permission preferences.

use gent_types::PolicyRecord;
use serde::{Deserialize, Serialize};

/// Negotiated capability for local permission-policy reads and explicit revisions.
pub const PERMISSION_POLICY_CAPABILITY: &str = "permission-policy-v1";

/// One finite permission-policy exchange. Provider execution is never part of this protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum PermissionPolicyFrame {
    Current {
        request_id: String,
        workspace_id: String,
    },
    Save {
        request_id: String,
        policy: PolicyRecord,
        /// Required when persisting the intentionally broad bypass mode.
        bypass_consent: bool,
    },
    CurrentPolicy {
        request_id: String,
        policy: Option<PolicyRecord>,
    },
    Saved {
        request_id: String,
        policy: PolicyRecord,
    },
}

#[cfg(test)]
mod tests {
    use super::{PERMISSION_POLICY_CAPABILITY, PermissionPolicyFrame};
    use gent_types::{PermissionMode, PolicyRecord, PolicyScope};
    use serde_json::json;

    #[test]
    fn save_frame_requires_explicit_bypass_consent_field() {
        let frame = json!({
            "type": "save", "body": {
                "requestId": "request-1", "policy": {
                    "policyId": "policy-1", "workspaceId": "workspace-1",
                    "scope": "providerPermissions", "revision": 1,
                    "mode": "bypass", "allowedTools": [], "allowedCategories": []
                }
            }
        });
        assert!(serde_json::from_value::<PermissionPolicyFrame>(frame).is_err());
        let valid = json!({
            "type": "current", "body": {
                "requestId": "request-1", "workspaceId": "workspace-1"
            }
        });
        assert!(serde_json::from_value::<PermissionPolicyFrame>(valid).is_ok());
        assert_eq!(PERMISSION_POLICY_CAPABILITY, "permission-policy-v1");
    }

    #[test]
    fn response_round_trips_the_new_revision() {
        let frame = PermissionPolicyFrame::Saved {
            request_id: "request-1".into(),
            policy: PolicyRecord {
                policy_id: "policy-1".into(),
                workspace_id: "workspace-1".into(),
                scope: PolicyScope::ProviderPermissions,
                revision: 1,
                mode: PermissionMode::Plan,
                allowed_tools: Vec::new(),
                allowed_categories: Vec::new(),
            },
        };
        assert_eq!(
            serde_json::from_value::<PermissionPolicyFrame>(serde_json::to_value(&frame).unwrap())
                .unwrap(),
            frame
        );
    }
}
