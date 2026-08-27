use gent_types::{AgentChatSession, AgentChatSessionId};

use crate::LedgerError;

pub trait AgentChatSessionLedger: Send + Sync {
    fn create_agent_chat_session(&self, session: &AgentChatSession) -> Result<(), LedgerError>;
    fn find_agent_chat_session(
        &self,
        session_id: &AgentChatSessionId,
    ) -> Result<Option<AgentChatSession>, LedgerError>;
    fn list_agent_chat_sessions(
        &self,
        workspace_id: &str,
    ) -> Result<Vec<AgentChatSession>, LedgerError>;
    fn attach_agent_chat_conversation(
        &self,
        session_id: &AgentChatSessionId,
        conversation_id: &str,
    ) -> Result<AgentChatSession, LedgerError>;
}
