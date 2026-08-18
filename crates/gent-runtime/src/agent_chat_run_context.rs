//! Read-only resolution of the frozen context boundary attached to a durable run.

use gent_ports::AgentChatRunContextReader;
use gent_types::{AgentChatConversationId, AgentChatRunContext, AgentChatRunId};

use crate::RuntimeError;

/// Provider-neutral runtime facade for exact run-context provenance.
#[derive(Clone, Debug)]
pub struct AgentChatRunContextService<L> {
    ledger: L,
}

impl<L> AgentChatRunContextService<L> {
    /// Creates a read-only context resolver without granting process or provider authority.
    #[must_use]
    pub fn new(ledger: L) -> Self {
        Self { ledger }
    }
}

impl<L: AgentChatRunContextReader> AgentChatRunContextService<L> {
    /// Resolves the immutable context policy and ordinal fixed for one exact run.
    ///
    /// # Errors
    /// Returns an error when the run does not belong to the requested conversation or its durable
    /// context provenance is unknown.
    pub fn resolve(
        &self,
        conversation_id: &AgentChatConversationId,
        run_id: &AgentChatRunId,
    ) -> Result<AgentChatRunContext, RuntimeError> {
        let result = self
            .ledger
            .read_agent_chat_run_context(conversation_id, run_id)?;
        if result.conversation_id != *conversation_id || result.run_id != *run_id {
            return Err(RuntimeError::Ledger(gent_ports::LedgerError::Invariant(
                "agent-chat context reader returned another run".into(),
            )));
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use gent_ports::{AgentChatRunContextReader, LedgerError};
    use gent_types::{
        AgentChatConversationId, AgentChatRunContext, AgentChatRunContextOrigin, AgentChatRunId,
        ContextPolicy,
    };

    use super::AgentChatRunContextService;

    #[derive(Clone)]
    struct Reader(AgentChatRunContext);

    impl AgentChatRunContextReader for Reader {
        fn read_agent_chat_run_context(
            &self,
            _: &AgentChatConversationId,
            _: &AgentChatRunId,
        ) -> Result<AgentChatRunContext, LedgerError> {
            Ok(self.0.clone())
        }
    }

    #[test]
    fn refuses_a_context_returned_for_another_run() {
        let service = AgentChatRunContextService::new(Reader(context("other")));
        assert!(
            service
                .resolve(
                    &AgentChatConversationId("conversation".into()),
                    &AgentChatRunId("run".into()),
                )
                .is_err()
        );
    }

    fn context(run_id: &str) -> AgentChatRunContext {
        AgentChatRunContext {
            conversation_id: AgentChatConversationId("conversation".into()),
            run_id: AgentChatRunId(run_id.into()),
            origin: AgentChatRunContextOrigin::SelectionSwitch,
            context_policy: ContextPolicy::Preserve,
            context_through_ordinal: 4,
        }
    }
}
