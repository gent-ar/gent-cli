use gent_protocol::LocalModelInstallState;
use gent_types::{
    AgentChatSelection, ConversationActivityFact, ConversationStatus, ConversationTimeline,
    NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
    PermissionDecisionRequest,
};
#[path = "view_metadata.rs"]
mod metadata;
pub(crate) use metadata::ConversationMetadata;
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct ConversationView {
    conversation_id: String,
    status: Option<ConversationStatus>,
    transcript: Vec<NormalizedTranscriptEvent>,
    activity: Vec<ConversationActivityFact>,
    timeline: Option<ConversationTimeline>,
    local_model_state: Option<LocalModelInstallState>,
    current_run_id: Option<String>,
    selection: Option<AgentChatSelection>,
    metadata: ConversationMetadata,
    pending_permission: Option<PermissionDecisionRequest>,
}

impl ConversationView {
    #[must_use]
    pub(crate) fn new(
        conversation_id: &str,
        status: Option<ConversationStatus>,
        transcript: Option<NormalizedTranscriptPage>,
    ) -> Self {
        let transcript = transcript
            .filter(|page| page.conversation_id == conversation_id)
            .map_or_else(Vec::new, |page| compact_transcript(page.events));
        Self {
            conversation_id: conversation_id.to_owned(),
            status: status.filter(|value| value.conversation_id == conversation_id),
            transcript,
            activity: Vec::new(),
            timeline: None,
            local_model_state: None,
            current_run_id: None,
            selection: None,
            metadata: ConversationMetadata::default(),
            pending_permission: None,
        }
    }
    #[must_use]
    pub(crate) fn with_current_run_id(mut self, current_run_id: Option<String>) -> Self {
        self.current_run_id = current_run_id.filter(|run_id| !run_id.trim().is_empty());
        self
    }
    #[must_use]
    pub(crate) fn with_selection(mut self, selection: Option<AgentChatSelection>) -> Self {
        self.selection = selection;
        self
    }
    #[must_use]
    pub(crate) fn with_activity(mut self, activity: Option<Vec<ConversationActivityFact>>) -> Self {
        self.activity = activity.unwrap_or_default();
        self
    }
    #[must_use]
    pub(crate) fn with_timeline(mut self, timeline: Option<ConversationTimeline>) -> Self {
        self.timeline = timeline.filter(|value| value.conversation_id == self.conversation_id);
        self
    }
    #[must_use]
    pub(crate) fn with_local_model_state(
        mut self,
        local_model_state: Option<LocalModelInstallState>,
    ) -> Self {
        self.local_model_state = local_model_state;
        self
    }
    #[must_use]
    pub(crate) fn with_metadata(
        mut self,
        title: Option<String>,
        recap: Option<String>,
        preview: Option<String>,
        workspace_id: Option<String>,
        workspace_path: Option<String>,
        mcp_server_count: u16,
        mcp_server_names: Vec<String>,
        automation_count: u16,
        automation_names: Vec<String>,
        automations: Vec<gent_types::AutomationDefinition>,
        automation_runs: Vec<gent_types::AutomationRunSummary>,
        forge_count: u16,
        forge_names: Vec<String>,
        changed_file_count: Option<u32>,
        git_branch: Option<String>,
    ) -> Self {
        self.metadata = ConversationMetadata {
            permission_mode: gent_types::PermissionMode::Default,
            title,
            recap,
            preview,
            workspace_id,
            workspace_path,
            mcp_server_count,
            mcp_server_names,
            automation_count,
            automation_names,
            automations,
            automation_runs,
            forge_count,
            forge_names,
            changed_file_count,
            git_branch,
        };
        self
    }
    #[must_use]
    pub(crate) fn with_pending_permission(
        mut self,
        pending_permission: Option<PermissionDecisionRequest>,
    ) -> Self {
        self.pending_permission = pending_permission;
        self
    }
    #[must_use]
    pub(crate) fn conversation_id(&self) -> &str {
        &self.conversation_id
    }
    #[must_use]
    pub(crate) fn status(&self) -> Option<&ConversationStatus> {
        self.status.as_ref()
    }
    #[must_use]
    pub(crate) fn transcript(&self) -> &[NormalizedTranscriptEvent] {
        &self.transcript
    }
    #[must_use]
    pub(crate) fn activity(&self) -> &[ConversationActivityFact] {
        &self.activity
    }
    #[must_use]
    pub(crate) fn timeline(&self) -> Option<&ConversationTimeline> {
        self.timeline.as_ref()
    }
    #[must_use]
    pub(crate) fn local_model_state(&self) -> Option<&LocalModelInstallState> {
        self.local_model_state.as_ref()
    }
    #[must_use]
    pub(crate) fn metadata(&self) -> &ConversationMetadata {
        &self.metadata
    }
    #[must_use]
    pub(crate) fn current_run_id(&self) -> Option<&str> {
        self.current_run_id.as_deref()
    }
    #[must_use]
    pub(crate) fn selection(&self) -> Option<&AgentChatSelection> {
        self.selection.as_ref()
    }

    #[must_use]
    pub(crate) fn pending_permission(&self) -> Option<&PermissionDecisionRequest> {
        self.pending_permission.as_ref()
    }
}

fn compact_transcript(events: Vec<NormalizedTranscriptEvent>) -> Vec<NormalizedTranscriptEvent> {
    let mut compacted = Vec::with_capacity(events.len());
    for event in events {
        if is_streamed_text(event.kind) && !event.is_partial {
            while compacted
                .last()
                .is_some_and(|previous: &NormalizedTranscriptEvent| {
                    previous.kind == event.kind
                        && previous.is_partial
                        && previous.run_id == event.run_id
                        && previous.turn_id == event.turn_id
                })
            {
                compacted.pop();
            }
        }
        compacted.push(event);
    }
    compacted
}

fn is_streamed_text(kind: NormalizedTranscriptKind) -> bool {
    matches!(
        kind,
        NormalizedTranscriptKind::AssistantMessage | NormalizedTranscriptKind::Thinking
    )
}

#[cfg(test)]
mod tests {
    use gent_types::{
        AgentChatEffort, AgentChatMode, AgentChatProvider, AgentChatSelection,
        NormalizedTranscriptEvent, NormalizedTranscriptKind, NormalizedTranscriptPage,
    };

    use super::ConversationView;

    #[test]
    fn view_rejects_data_for_another_conversation() {
        let view = ConversationView::new(
            "selected",
            None,
            Some(NormalizedTranscriptPage {
                conversation_id: "other".into(),
                events: vec![NormalizedTranscriptEvent {
                    cursor: 1,
                    event_id: "event".into(),
                    turn_id: "turn".into(),
                    run_id: "run".into(),
                    kind: NormalizedTranscriptKind::AssistantMessage,
                    text: "must not render".into(),
                    is_partial: false,
                }],
                next_after_cursor: None,
            }),
        );
        assert_eq!(view.conversation_id(), "selected");
        assert!(view.transcript().is_empty());
    }

    #[test]
    fn final_assistant_event_replaces_its_partial_stream_in_the_initial_page() {
        let mut partial = event(1, "hel", true);
        let final_event = event(2, "hello", false);
        partial.kind = NormalizedTranscriptKind::AssistantMessage;
        let view = ConversationView::new(
            "selected",
            None,
            Some(NormalizedTranscriptPage {
                conversation_id: "selected".into(),
                events: vec![partial, final_event],
                next_after_cursor: None,
            }),
        );
        assert_eq!(view.transcript().len(), 1);
        assert_eq!(view.transcript()[0].text, "hello");
    }

    #[test]
    fn final_thinking_replaces_only_thinking_deltas_and_keeps_the_answer() {
        let mut thinking_partial = event(1, "consider", true);
        thinking_partial.kind = NormalizedTranscriptKind::Thinking;
        let mut thinking_final = event(2, "considered", false);
        thinking_final.kind = NormalizedTranscriptKind::Thinking;
        let answer = event(3, "answer", false);
        let view = ConversationView::new(
            "selected",
            None,
            Some(NormalizedTranscriptPage {
                conversation_id: "selected".into(),
                events: vec![thinking_partial, thinking_final, answer],
                next_after_cursor: None,
            }),
        );
        assert_eq!(view.transcript().len(), 2);
        assert_eq!(
            view.transcript()[0].kind,
            NormalizedTranscriptKind::Thinking
        );
        assert_eq!(view.transcript()[0].text, "considered");
        assert_eq!(
            view.transcript()[1].kind,
            NormalizedTranscriptKind::AssistantMessage
        );
        assert_eq!(view.transcript()[1].text, "answer");
    }

    #[test]
    fn view_keeps_the_durable_current_selection_for_terminal_controls() {
        let selection = AgentChatSelection {
            provider: AgentChatProvider::Codex,
            model: "gpt-5.6".into(),
            effort: AgentChatEffort::High,
            mode: AgentChatMode::Agent,
        };
        let view =
            ConversationView::new("selected", None, None).with_selection(Some(selection.clone()));
        assert_eq!(view.selection(), Some(&selection));
    }
    fn event(cursor: u64, text: &str, is_partial: bool) -> NormalizedTranscriptEvent {
        NormalizedTranscriptEvent {
            cursor,
            event_id: format!("event-{cursor}"),
            turn_id: "turn".into(),
            run_id: "run".into(),
            kind: NormalizedTranscriptKind::AssistantMessage,
            text: text.into(),
            is_partial,
        }
    }
}
