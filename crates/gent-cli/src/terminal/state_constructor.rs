use super::{AgentChatSelection, AgentChatSession, UiState};
use gent_protocol::DEFAULT_LOCAL_MODEL_ID;
use gent_types::{AgentChatEffort, AgentChatMode, AgentChatProvider};
use std::collections::BTreeMap;

impl UiState {
    #[must_use]
    pub(crate) fn with_chat_input(mut self, chat_enabled: bool) -> Self {
        self.chat_enabled = chat_enabled;
        self
    }
    #[must_use]
    pub(crate) fn with_sessions(mut self, sessions: Vec<AgentChatSession>) -> Self {
        self.selected_session = (!sessions.is_empty()).then_some(0);
        self.sessions = sessions;
        self
    }
    #[must_use]
    pub(crate) fn with_show_thinking(mut self, show_thinking: bool) -> Self {
        self.show_thinking = show_thinking;
        self
    }
    #[must_use]
    pub(crate) fn with_local_model_ids(mut self, local_model_ids: Vec<String>) -> Self {
        self.local_model_ids = local_model_ids;
        self
    }
    #[must_use]
    pub(crate) fn new(conversations: Vec<super::ConversationListItem>) -> Self {
        let selected = (!conversations.is_empty()).then_some(0);
        Self {
            conversations,
            selected,
            chat_enabled: false,
            input: String::new(),
            scroll_offset: 0,
            attachments: Vec::new(),
            metadata: BTreeMap::new(),
            sessions: Vec::new(),
            session_focus: false,
            selected_session: None,
            selection: AgentChatSelection {
                provider: AgentChatProvider::Claurst,
                model: DEFAULT_LOCAL_MODEL_ID.into(),
                effort: AgentChatEffort::Medium,
                mode: AgentChatMode::Agent,
            },
            context_policy: super::ContextPolicy::Preserve,
            parent_run_id: None,
            view: None,
            notice: None,
            help_visible: false,
            activity_visible: false,
            documents: Vec::new(),
            documents_visible: false,
            document_cursor: 0,
            templates: Vec::new(),
            templates_visible: false,
            template_cursor: 0,
            automations_visible: false,
            automation_cursor: 0,
            selection_picker: None,
            selection_picker_index: 0,
            local_model_ids: Vec::new(),
            show_thinking: std::env::var("GENT_SHOW_THINKING")
                .is_ok_and(|value| matches!(value.as_str(), "1" | "true")),
            awaiting_turn: false,
            conversation_filter: String::new(),
        }
    }
}
