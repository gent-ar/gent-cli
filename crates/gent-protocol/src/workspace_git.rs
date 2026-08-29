//! Negotiated local IPC for read-only workspace git status and worktrees.

use gent_types::WorkspaceGitReport;
use serde::{Deserialize, Serialize};

/// Negotiated capability for local workspace git status and sub-repository discovery.
pub const WORKSPACE_GIT_CAPABILITY: &str = "workspace-git-v1";

/// One finite workspace-git exchange. No mutation of the repository is part of this protocol.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkspaceGitFrame {
    StatusRequest {
        request_id: String,
        workspace_id: String,
    },
    Status {
        request_id: String,
        workspace_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        report: Option<WorkspaceGitReport>,
    },
    SubReposRequest {
        request_id: String,
        workspace_id: String,
    },
    SubRepos {
        request_id: String,
        workspace_id: String,
        canonical_paths: Vec<String>,
    },
    ResolveRequest {
        request_id: String,
        workspace_path: String,
    },
    Resolved {
        request_id: String,
        workspace_id: String,
        canonical_path: String,
    },
}

#[cfg(test)]
mod tests {
    use super::{WORKSPACE_GIT_CAPABILITY, WorkspaceGitFrame};
    use serde_json::json;

    #[test]
    fn status_response_omits_the_report_when_not_a_git_repository() {
        let frame = WorkspaceGitFrame::Status {
            request_id: "request-1".into(),
            workspace_id: "workspace-1".into(),
            report: None,
        };
        assert_eq!(
            serde_json::to_value(&frame).unwrap(),
            json!({
                "type": "status",
                "body": { "requestId": "request-1", "workspaceId": "workspace-1" }
            })
        );
        assert_eq!(WORKSPACE_GIT_CAPABILITY, "workspace-git-v1");
    }

    #[test]
    fn frame_rejects_unknown_fields() {
        let frame = json!({
            "type": "statusRequest",
            "body": { "requestId": "request-1", "workspaceId": "workspace-1", "extra": true }
        });
        assert!(serde_json::from_value::<WorkspaceGitFrame>(frame).is_err());
    }
}
