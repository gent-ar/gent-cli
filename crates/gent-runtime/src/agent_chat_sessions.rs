use gent_ports::AgentChatSessionLedger;
use gent_types::{AgentChatSession, AgentChatSessionId};

use crate::RuntimeError;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum AgentChatSessionAuthority {
    #[default]
    Observer,
    Approved,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AgentChatSessionResult {
    DeniedObserver,
    Missing,
    Session(AgentChatSession),
    Sessions(Vec<AgentChatSession>),
}

#[derive(Clone, Debug)]
pub struct AgentChatSessionService<L> {
    ledger: L,
    authority: AgentChatSessionAuthority,
}

impl<L> AgentChatSessionService<L> {
    pub fn new(ledger: L, authority: AgentChatSessionAuthority) -> Self {
        Self { ledger, authority }
    }
}

impl<L: AgentChatSessionLedger> AgentChatSessionService<L> {
    pub fn create(
        &self,
        session: AgentChatSession,
    ) -> Result<AgentChatSessionResult, RuntimeError> {
        if self.authority != AgentChatSessionAuthority::Approved {
            return Ok(AgentChatSessionResult::DeniedObserver);
        }
        self.ledger.create_agent_chat_session(&session)?;
        Ok(AgentChatSessionResult::Session(session))
    }

    pub fn list(&self, workspace_id: &str) -> Result<AgentChatSessionResult, RuntimeError> {
        if self.authority != AgentChatSessionAuthority::Approved {
            return Ok(AgentChatSessionResult::DeniedObserver);
        }
        Ok(AgentChatSessionResult::Sessions(
            self.ledger.list_agent_chat_sessions(workspace_id)?,
        ))
    }

    pub fn get(
        &self,
        session_id: &AgentChatSessionId,
    ) -> Result<AgentChatSessionResult, RuntimeError> {
        if self.authority != AgentChatSessionAuthority::Approved {
            return Ok(AgentChatSessionResult::DeniedObserver);
        }
        Ok(self.ledger.find_agent_chat_session(session_id)?.map_or(
            AgentChatSessionResult::Missing,
            AgentChatSessionResult::Session,
        ))
    }

    pub fn attach(
        &self,
        session_id: &AgentChatSessionId,
        conversation_id: &str,
    ) -> Result<AgentChatSessionResult, RuntimeError> {
        if self.authority != AgentChatSessionAuthority::Approved {
            return Ok(AgentChatSessionResult::DeniedObserver);
        }
        Ok(AgentChatSessionResult::Session(
            self.ledger
                .attach_agent_chat_conversation(session_id, conversation_id)?,
        ))
    }
}
