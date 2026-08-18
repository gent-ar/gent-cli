//! Child-run history projection for the dormant public-driver composition.

use gent_ports::{AgentChatRunContextReader, ConversationContentReader, TranscriptLedger};
use gent_runtime::{
    AgentChatRunContextService, ConversationContextArtifactService, ConversationContextRequest,
    RuntimeError,
};
use gent_types::{AgentChatConversationId, AgentChatRunId, FrozenConversationContext};

/// Joins a durable run boundary to the bounded provider-neutral history artifact.
#[derive(Debug)]
pub(crate) struct RunContextProjection<L> {
    runs: AgentChatRunContextService<L>,
    artifacts: ConversationContextArtifactService<L>,
}

impl<L: Clone> RunContextProjection<L> {
    pub(crate) fn new(ledger: L) -> Self {
        Self {
            runs: AgentChatRunContextService::new(ledger.clone()),
            artifacts: ConversationContextArtifactService::new(ledger),
        }
    }
}

impl<L> RunContextProjection<L>
where
    L: AgentChatRunContextReader + ConversationContentReader + TranscriptLedger,
{
    /// Returns fresh input only for durable children; roots may resume only themselves.
    pub(crate) fn fresh_context_for_child(
        &self,
        conversation_id: &str,
        run_id: &str,
    ) -> Result<Option<FrozenConversationContext>, RuntimeError> {
        let conversation_id = AgentChatConversationId(conversation_id.into());
        let run_id = AgentChatRunId(run_id.into());
        let boundary = self.runs.resolve(&conversation_id, &run_id)?;
        if !boundary.requires_fresh_provider_session() {
            return Ok(None);
        }
        self.artifacts
            .project(&ConversationContextRequest {
                conversation_id,
                context_policy: boundary.context_policy,
                context_through_ordinal: boundary.context_through_ordinal,
            })
            .map(Some)
    }
}
