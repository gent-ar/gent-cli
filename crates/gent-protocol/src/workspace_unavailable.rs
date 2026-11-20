use serde::{Deserialize, Serialize};

pub const WORKSPACE_UNAVAILABLE: &str = "workspaceUnavailable";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WorkspaceUnavailable {
    pub path: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum WorkspaceRejectionFrame {
    Error {
        code: String,
        message: String,
        workspace: WorkspaceUnavailable,
    },
}

#[cfg(test)]
mod tests {
    use super::{WORKSPACE_UNAVAILABLE, WorkspaceRejectionFrame, WorkspaceUnavailable};
    use serde_json::json;

    #[test]
    fn a_missing_workspace_rejection_names_the_folder() {
        let frame = WorkspaceRejectionFrame::Error {
            code: WORKSPACE_UNAVAILABLE.into(),
            message: "the workspace folder is gone".into(),
            workspace: WorkspaceUnavailable {
                path: "/gone".into(),
            },
        };
        assert_eq!(
            serde_json::to_value(&frame).unwrap(),
            json!({"type": "error", "body": {
                "code": "workspaceUnavailable",
                "message": "the workspace folder is gone",
                "workspace": {"path": "/gone"}
            }})
        );
    }
}
