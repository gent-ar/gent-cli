use serde::{Deserialize, Serialize};

pub const WORKSPACE_DOCUMENTS_CAPABILITY: &str = "workspace-documents-v1";

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum WorkspaceDocumentGroup {
    Project,
    Gent,
    Docs,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceDocumentRecord {
    pub document_id: String,
    pub group: WorkspaceDocumentGroup,
    pub relative_path: String,
    pub absolute_path: String,
    pub byte_len: u64,
    pub modified_unix_seconds: u64,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(
    tag = "type",
    content = "body",
    rename_all = "camelCase",
    deny_unknown_fields
)]
pub enum WorkspaceDocumentsFrame {
    List {
        request_id: String,
        workspace_id: String,
    },
    Listed {
        request_id: String,
        workspace_id: String,
        documents: Vec<WorkspaceDocumentRecord>,
    },
}

impl WorkspaceDocumentsFrame {
    /// # Errors
    ///
    /// Returns an error when the frame's request or workspace data is invalid.
    pub fn validate(&self) -> Result<(), &'static str> {
        let (request_id, workspace_id) = match self {
            Self::List {
                request_id,
                workspace_id,
            }
            | Self::Listed {
                request_id,
                workspace_id,
                ..
            } => (request_id, workspace_id),
        };
        if request_id.is_empty()
            || request_id.len() > 128
            || request_id.chars().any(char::is_control)
        {
            return Err("workspace document request identifier is invalid");
        }
        if workspace_id.is_empty()
            || workspace_id.len() > 128
            || workspace_id.chars().any(char::is_control)
        {
            return Err("workspace identifier is invalid");
        }
        if let Self::Listed { documents, .. } = self {
            if documents.len() > 256 {
                return Err("too many workspace documents");
            }
            if documents
                .iter()
                .any(|item| item.relative_path.is_empty() || item.absolute_path.is_empty())
            {
                return Err("workspace document path is invalid");
            }
        }
        Ok(())
    }
}
