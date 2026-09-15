//! Child-run history projection for the dormant public-driver composition.

use gent_ports::{AgentChatRunContextReader, ConversationContentReader, Ledger, TranscriptLedger};
use gent_runtime::{
    AgentChatRunContextService, ConversationContextArtifactService, ConversationContextRequest,
    RuntimeError,
};
use gent_types::{
    AgentChatConversationId, AgentChatRunId, ConversationMessage, FrozenConversationContext,
};

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
    pub(crate) fn fresh_context_before_message(
        &self,
        conversation_id: &str,
        message_id: &str,
    ) -> Result<FrozenConversationContext, RuntimeError> {
        self.artifacts
            .project_before_message(AgentChatConversationId(conversation_id.into()), message_id)
    }

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

impl<L, D, R> super::PublicDriversRuntime<L, D, R>
where
    L: Ledger + AgentChatRunContextReader + ConversationContentReader + TranscriptLedger,
{
    pub(crate) fn launch_context(
        &self,
        message: &ConversationMessage,
    ) -> Result<Option<FrozenConversationContext>, RuntimeError> {
        if self
            .ledger
            .find_run_session_binding(&message.run_id)?
            .is_some()
        {
            return Ok(None);
        }
        match self
            .contexts
            .fresh_context_for_child(&message.conversation_id, &message.run_id)?
        {
            Some(context) => Ok(Some(context)),
            None => self
                .contexts
                .fresh_context_before_message(&message.conversation_id, &message.message_id)
                .map(Some),
        }
    }

    pub(crate) fn interrupted_reply_before(
        &self,
        message: &ConversationMessage,
    ) -> Result<Option<String>, RuntimeError> {
        let context = self
            .contexts
            .fresh_context_before_message(&message.conversation_id, &message.message_id)?;
        let Some(previous) = context
            .entries
            .last()
            .filter(|entry| entry.run_id == message.run_id)
        else {
            return Ok(None);
        };
        let Some(reply) = context.transcript_events.iter().rev().find(|event| {
            event.turn_id == previous.turn_id
                && event.kind == gent_types::NormalizedTranscriptKind::AssistantMessage
        }) else {
            return Ok(None);
        };
        if !reply
            .event_id
            .starts_with(gent_types::INTERRUPTED_REPLY_EVENT_PREFIX)
        {
            return Ok(None);
        }
        let settled_unfinished = self
            .ledger
            .normalized_transcript_page(
                &AgentChatConversationId(message.conversation_id.clone()),
                reply.cursor.saturating_sub(1),
                1,
            )?
            .events
            .first()
            .is_some_and(|stored| stored.event_id == reply.event_id);
        Ok(settled_unfinished.then(|| reply.text.clone()))
    }
}
