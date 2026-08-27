use super::{AgentChatSession, UiEffect, UiState};
use gent_types::AgentChatSessionId;

impl UiState {
    pub(crate) fn sessions(&self) -> &[AgentChatSession] {
        &self.sessions
    }

    pub(super) fn select_session(
        &mut self,
        select: impl FnOnce(usize, usize) -> usize,
    ) -> UiEffect {
        if let Some(index) = self.selected_session {
            self.selected_session = Some(select(index, self.sessions.len()));
        }
        UiEffect::Continue
    }

    pub(super) fn open_session(&mut self) -> UiEffect {
        self.session_focus = false;
        let Some(session) = self
            .selected_session
            .and_then(|index| self.sessions.get(index))
        else {
            return UiEffect::Continue;
        };
        let Some(conversation_id) = session.conversation_ids.last().cloned() else {
            return UiEffect::Continue;
        };
        if self.select_conversation(&conversation_id) {
            UiEffect::Refresh(conversation_id)
        } else {
            UiEffect::Continue
        }
    }

    pub(super) fn toggle_session_focus(&mut self) -> UiEffect {
        if self.sessions.is_empty() {
            self.notice = Some("There are no sessions in this workspace yet.".into());
        } else {
            self.session_focus = !self.session_focus;
        }
        UiEffect::Continue
    }

    pub(super) fn focused_session_id(&self) -> Option<AgentChatSessionId> {
        self.session_focus
            .then(|| self.selected_session)
            .flatten()
            .and_then(|index| self.sessions.get(index))
            .map(|session| session.session_id.clone())
    }

    pub(super) fn create_session(&mut self, name: &str) -> UiEffect {
        let Some(conversation) = self.selected() else {
            self.notice = Some("Select a conversation before creating a session.".into());
            return UiEffect::Continue;
        };
        let Some(workspace_id) = self
            .metadata(&conversation.conversation_id)
            .and_then(|metadata| metadata.workspace_id.clone())
        else {
            self.notice = Some("The selected conversation has no workspace session scope.".into());
            return UiEffect::Continue;
        };
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |value| value.as_secs().try_into().unwrap_or(i64::MAX));
        UiEffect::CreateSession(AgentChatSession {
            session_id: AgentChatSessionId(format!("session-{}", uuid::Uuid::new_v4().simple())),
            workspace_id,
            name: name.into(),
            conversation_ids: vec![conversation.conversation_id.clone()],
            created_at,
            updated_at: created_at,
        })
    }

    pub(crate) fn add_session(&mut self, session: AgentChatSession) {
        self.selected_session = Some(self.sessions.len());
        self.sessions.push(session);
        self.notice = Some("Session created for the selected conversation.".into());
    }
}
