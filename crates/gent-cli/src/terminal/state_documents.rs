use super::{UiEffect, UiState};
use gent_protocol::WorkspaceDocumentRecord;
use std::path::PathBuf;

pub(super) fn list(state: &mut UiState, argument: &str) -> UiEffect {
    state.input.clear();
    let Some(workspace_id) = state
        .selected()
        .and_then(|item| state.metadata(&item.conversation_id))
        .and_then(|metadata| metadata.workspace_id.clone())
    else {
        state.notice = Some("The selected conversation has no workspace documents.".into());
        return UiEffect::Continue;
    };
    UiEffect::ListDocuments {
        workspace_id,
        attach_id: (!argument.is_empty()).then(|| argument.to_owned()),
    }
}

impl UiState {
    pub(super) fn clear_documents(&mut self) {
        self.documents.clear();
        self.documents_visible = false;
        self.document_cursor = 0;
    }

    pub(crate) fn set_documents(
        &mut self,
        documents: Vec<WorkspaceDocumentRecord>,
        attach_id: Option<String>,
    ) {
        self.documents = documents;
        self.document_cursor = 0;
        self.documents_visible = attach_id.is_none();
        if let Some(id) = attach_id {
            if let Some(path) = self
                .documents
                .iter()
                .find(|item| item.document_id == id)
                .map(|item| PathBuf::from(&item.absolute_path))
            {
                self.stage_document(path);
                return;
            }
            self.notice = Some(format!("Document not found: {id}"));
        }
    }

    pub(crate) fn stage_document(&mut self, path: PathBuf) {
        if self.attachments.iter().any(|item| item == &path) {
            self.notice = Some("That document is already attached.".into());
            return;
        }
        if !path.is_file() {
            self.notice = Some("Document path is no longer available.".into());
            return;
        }
        self.attachments.push(path);
        self.notice = Some("Document staged for the next prompt.".into());
    }

    pub(crate) fn document_move(&mut self, next: bool) {
        if self.documents_visible && !self.documents.is_empty() {
            let last = self.documents.len() - 1;
            self.document_cursor = if next {
                (self.document_cursor + 1).min(last)
            } else {
                self.document_cursor.saturating_sub(1)
            };
        }
    }

    pub(crate) fn document_submit(&mut self) -> bool {
        if !self.documents_visible {
            return false;
        }
        if let Some(document) = self.documents.get(self.document_cursor) {
            self.stage_document(PathBuf::from(&document.absolute_path));
        }
        self.documents_visible = false;
        true
    }
}
