use gent_protocol::LocalModelInstallState;
use gent_types::{
    ConversationActivityFact, ConversationStatus, ConversationTimeline, NormalizedTranscriptEvent,
    NormalizedTranscriptKind, PermissionDecisionRequest,
};
use std::collections::BTreeMap;

use super::{ConversationMetadata, ConversationView, UiState};

impl UiState {
    #[must_use]
    pub(crate) fn permission_mode(&self) -> gent_types::PermissionMode {
        self.selected()
            .and_then(|item| self.metadata.get(&item.conversation_id))
            .map_or(gent_types::PermissionMode::Default, |metadata| {
                metadata.permission_mode
            })
    }

    #[must_use]
    pub(crate) const fn scroll_offset(&self) -> u16 {
        self.scroll_offset
    }

    #[must_use]
    pub(crate) fn with_view(mut self, view: Option<ConversationView>) -> Self {
        self.view = view.filter(|view| {
            self.selected()
                .is_some_and(|item| item.conversation_id == view.conversation_id())
        });
        self.parent_run_id = self.view.as_ref().and_then(run_id);
        if let Some(selection) = self.view.as_ref().and_then(ConversationView::selection) {
            self.set_selection(selection.clone());
        }
        self
    }

    #[must_use]
    pub(crate) fn with_metadata(
        mut self,
        metadata: BTreeMap<String, ConversationMetadata>,
    ) -> Self {
        self.metadata = metadata;
        self
    }

    #[must_use]
    pub(crate) fn metadata(&self, conversation_id: &str) -> Option<&ConversationMetadata> {
        self.metadata.get(conversation_id)
    }

    #[must_use]
    pub(crate) fn attachment_count(&self) -> usize {
        self.attachments.len()
    }

    #[must_use]
    pub(crate) fn selected_workspace_path(&self) -> Option<&str> {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .and_then(|metadata| metadata.workspace_path.as_deref())
    }

    #[must_use]
    pub(crate) fn selected_workspace_id(&self) -> Option<&str> {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .and_then(|metadata| metadata.workspace_id.as_deref())
    }

    #[must_use]
    pub(crate) fn selected_mcp_server_count(&self) -> u16 {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .map_or(0, |metadata| metadata.mcp_server_count)
    }

    #[must_use]
    pub(crate) fn selected_mcp_server_names(&self) -> &[String] {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .map_or(&[], |metadata| &metadata.mcp_server_names)
    }

    #[must_use]
    pub(crate) fn selected_automation_count(&self) -> u16 {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .map_or(0, |metadata| metadata.automation_count)
    }

    #[must_use]
    pub(crate) fn selected_automation_names(&self) -> &[String] {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .map_or(&[], |metadata| &metadata.automation_names)
    }

    #[must_use]
    pub(crate) fn selected_automations(&self) -> &[gent_types::AutomationDefinition] {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .map_or(&[], |metadata| &metadata.automations)
    }

    #[must_use]
    pub(crate) fn selected_automation_runs(&self) -> &[gent_types::AutomationRunSummary] {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .map_or(&[], |metadata| &metadata.automation_runs)
    }

    #[must_use]
    pub(crate) fn selected_forge_count(&self) -> u16 {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .map_or(0, |metadata| metadata.forge_count)
    }

    #[must_use]
    pub(crate) fn selected_forge_names(&self) -> &[String] {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .map_or(&[], |metadata| &metadata.forge_names)
    }

    #[must_use]
    pub(crate) fn selected_changed_file_count(&self) -> Option<u32> {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .and_then(|metadata| metadata.changed_file_count)
    }

    #[must_use]
    pub(crate) fn selected_git_branch(&self) -> Option<&str> {
        self.selected()
            .and_then(|item| self.metadata(&item.conversation_id))
            .and_then(|metadata| metadata.git_branch.as_deref())
    }

    #[cfg(test)]
    #[must_use]
    pub(crate) fn with_status(self, status: Option<ConversationStatus>) -> Self {
        let view = status.map(|status| {
            let conversation_id = status.conversation_id.clone();
            ConversationView::new(&conversation_id, Some(status), None)
        });
        self.with_view(view)
    }

    #[must_use]
    pub(crate) fn selected_status(&self) -> Option<&ConversationStatus> {
        self.view
            .as_ref()
            .and_then(|view| view.status())
            .filter(|status| {
                self.selected()
                    .is_some_and(|item| item.conversation_id == status.conversation_id)
            })
    }

    #[must_use]
    pub(crate) fn selected_transcript(&self) -> &[NormalizedTranscriptEvent] {
        self.view
            .as_ref()
            .filter(|view| {
                self.selected()
                    .is_some_and(|item| item.conversation_id == view.conversation_id())
            })
            .map_or(&[], ConversationView::transcript)
    }

    #[must_use]
    pub(crate) fn selected_activity(&self) -> &[ConversationActivityFact] {
        self.view
            .as_ref()
            .filter(|view| {
                self.selected()
                    .is_some_and(|item| item.conversation_id == view.conversation_id())
            })
            .map_or(&[], ConversationView::activity)
    }

    #[must_use]
    pub(crate) fn selected_timeline(&self) -> Option<&ConversationTimeline> {
        self.view
            .as_ref()
            .filter(|view| {
                self.selected()
                    .is_some_and(|item| item.conversation_id == view.conversation_id())
            })
            .and_then(ConversationView::timeline)
    }

    #[must_use]
    pub(crate) fn selected_local_model_state(&self) -> Option<&LocalModelInstallState> {
        self.view
            .as_ref()
            .filter(|view| {
                self.selected()
                    .is_some_and(|item| item.conversation_id == view.conversation_id())
            })
            .and_then(ConversationView::local_model_state)
    }

    #[must_use]
    pub(crate) fn selected_pending_permission(&self) -> Option<&PermissionDecisionRequest> {
        self.view
            .as_ref()
            .filter(|view| {
                self.selected()
                    .is_some_and(|item| item.conversation_id == view.conversation_id())
            })
            .and_then(ConversationView::pending_permission)
    }

    pub(crate) fn apply_view(&mut self, view: ConversationView) {
        if self
            .selected()
            .is_some_and(|item| item.conversation_id == view.conversation_id())
        {
            if view.transcript().iter().any(|event| {
                event.kind == NormalizedTranscriptKind::AssistantMessage && !event.is_partial
            }) {
                self.finish_awaiting_turn();
            }
            if let Some(status) = view.status() {
                self.update_selected_run_count(
                    view.conversation_id(),
                    status.runs.len().try_into().unwrap_or(u32::MAX),
                );
            }
            self.parent_run_id = run_id(&view);
            if let Some(selection) = view.selection() {
                self.set_selection(selection.clone());
            }
            let permission_mode = self
                .metadata
                .get(view.conversation_id())
                .map_or(gent_types::PermissionMode::Default, |metadata| {
                    metadata.permission_mode
                });
            let mut metadata = view.metadata().clone();
            metadata.permission_mode = permission_mode;
            self.metadata
                .insert(view.conversation_id().into(), metadata);
            if settled_prompt(&view)
                && self
                    .notice
                    .as_deref()
                    .is_some_and(|notice| notice.starts_with("Gent is "))
            {
                self.notice = Some("Ready for your next message.".into());
            }
            self.view = Some(view);
        }
    }
}

fn settled_prompt(view: &ConversationView) -> bool {
    view.transcript()
        .iter()
        .any(|event| event.kind == NormalizedTranscriptKind::AssistantMessage && !event.is_partial)
}

fn run_id(view: &ConversationView) -> Option<String> {
    if let Some(run_id) = view.current_run_id() {
        return Some(run_id.into());
    }
    view.status()
        .and_then(|status| match status.runs.as_slice() {
            [run] if !run.run_id.is_empty() => Some(run.run_id.clone()),
            _ => None,
        })
}
